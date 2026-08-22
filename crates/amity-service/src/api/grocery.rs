// api/grocery.rs — HTTP handlers for the Grocery List/Item API.
//
// Endpoints:
//   POST   /api/v1/grocery-lists                  — create a new list
//   GET    /api/v1/grocery-lists                   — list every list
//   GET    /api/v1/grocery-lists/{id}              — fetch one list
//   POST   /api/v1/grocery-lists/{id}/items         — add an item manually
//   GET    /api/v1/grocery-lists/{id}/items         — list a list's items
//   POST   /api/v1/grocery-lists/{id}/generate      — generate additions from
//                                                      planned meals
//   PATCH  /api/v1/grocery-items/{id}               — toggle checked
//   DELETE /api/v1/grocery-items/{id}               — remove an item
//
// Handler shape mirrors api/meal.rs and api/event.rs: `State(state):
// State<AppState>`, `Json(req): Json<...>` request bodies, `(StatusCode,
// Json(resp))` success responses, and the same `unprocessable`/`bad_request`/
// `not_found` helpers (kept local to this module).
//
// Error mapping (matching api/meal.rs):
//   400 Bad Request    — a path parameter is not a valid UUID, or `?from=`/
//                        `?to=` on generate is present but not YYYY-MM-DD (or
//                        only one of the pair is given).
//   404 Not Found      — a valid id names no list (or no item, for the PATCH/
//                        DELETE item endpoints).
//   422 Unprocessable  — a body field is invalid (blank name).
//   500 Internal Error — an unexpected storage failure (logged, not leaked).
//
// ── The generation endpoint's contract ──────────────────────────────────────
//
// `POST /api/v1/grocery-lists/{id}/generate` is this phase's payoff: it turns
// planned meals into grocery additions while never touching what is already
// on the list (see `amity_core::grocery::plan_grocery_additions`'s doc
// comment for the no-clobber invariants this relies on). It returns ONLY the
// newly-added items — not the full list — because the caller already knows
// what was on the list before calling (it is the one who put it there), and
// returning just the delta makes "nothing new to add" a visibly empty
// `added: []` rather than forcing the client to diff two full-list snapshots
// to find out what changed. A client that wants the full list makes a
// separate `GET .../items` call, which this endpoint's response does not
// duplicate.
//
// There is no PATCH for a whole list (renaming, etc) — only items are
// mutable via PATCH — matching the brief's fixed scope for this slice.
//
// ── Response shape conventions ───────────────────────────────────────────
//
// List and item listings (`list_grocery_lists_handler`,
// `list_grocery_items_handler`) return a bare JSON array, matching
// `list_events_handler`'s convention — unlike Calendars' own
// `{ "calendars": [...] }` wrapper. A single list or item is always the flat
// object shape (`GroceryListResponse` / `GroceryItemResponse`), never wrapped.
//
// PATCH/DELETE on an item return only a small confirmation body (id plus the
// field that changed), not the full item — see `patch_grocery_item`'s doc
// comment for why (there is no single-item fetch in the storage layer).

// axum plumbing.
// The JSON extractor/response wrapper used by every handler in this module.
use axum::Json;
// `Path` extracts `{id}`; `Query` extracts `?checked=`-style params (here,
// `/generate`'s `?from=&to=`); `State` injects the shared `AppState`.
use axum::extract::{Path, Query, State};
// HTTP status codes used in the response tuples below.
use axum::http::StatusCode;
// Sealed trait implemented by tuples, `Json<T>`, `StatusCode`, … for dispatch.
use axum::response::IntoResponse;
// Request/response (de)serialisation.
// `Deserialize` for request bodies; `Serialize` for response bodies.
use serde::{Deserialize, Serialize};
// Builds the small `{ "error": ... }` bodies for the 4xx helpers below.
use serde_json::json;
// Datetimes: OffsetDateTime for `created_at`/generation's `now`; Rfc3339 for
// the wire; Date/Duration for the generate endpoint's date-range resolution.
use time::format_description::well_known::Rfc3339;
use time::{Date, Duration, OffsetDateTime};

// Domain types: the grocery entities plus the pure generation function.
// The two builders, their entities, the error type, and the pure function
// this endpoint's whole payoff rests on.
use amity_core::grocery::{
    GroceryError, GroceryItem, GroceryItemBuilder, GroceryList, GroceryListBuilder,
    plan_grocery_additions,
};
// Typed ids for the list and item path parameters.
use amity_core::ids::{GroceryItemId, GroceryListId};

// Storage.
// The grocery repository functions this module calls directly.
use amity_storage::grocery::{
    delete_grocery_item, fetch_grocery_list, insert_grocery_item, insert_grocery_items,
    insert_grocery_list, list_grocery_items, list_grocery_lists, set_grocery_item_checked,
};
// The generate endpoint's two other inputs: meals in range, and the pantry.
use amity_storage::meal::list_meals_in_range;
use amity_storage::pantry::list_pantry_items;

// Shared app state (the pool).
use crate::AppState;

// ─── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/grocery-lists`.
#[derive(Debug, Deserialize)]
pub struct CreateGroceryListRequest {
    /// The list's display name (e.g. "Groceries"); must be non-empty after
    /// trimming. There is no service-layer default — the client always sends
    /// one (the hub's own "New list" flow can supply "Groceries" itself).
    pub name: String,
}

/// Request body for `POST /api/v1/grocery-lists/{id}/items` — a manual add.
///
/// Items created here always carry `source: "manual"` — meal-generated items
/// only ever come from the `/generate` endpoint.
#[derive(Debug, Deserialize)]
pub struct CreateGroceryItemRequest {
    /// The item's name (e.g. "eggs"); must be non-empty after trimming.
    pub name: String,
    /// Freetext quantity, if the household chose to record one.
    pub qty: Option<String>,
    /// Optional free-form category (e.g. "produce") for grouping in the UI.
    pub category: Option<String>,
}

/// Request body for `PATCH /api/v1/grocery-items/{id}`.
///
/// The only mutable field is `checked` — toggling whether the item has been
/// bought. There is no rename/re-quantity endpoint in this slice.
#[derive(Debug, Deserialize)]
pub struct PatchGroceryItemRequest {
    /// The new checked-off state.
    pub checked: bool,
}

/// Query parameters for `POST /api/v1/grocery-lists/{id}/generate`.
///
/// `from`/`to` must both be present (an inclusive date range) or both absent
/// (default: the current Monday-Sunday week, computed the same way
/// `GET /api/v1/week` computes its default window) — half a pair is a 400.
#[derive(Debug, Deserialize)]
pub struct GenerateQuery {
    /// Inclusive lower bound (`YYYY-MM-DD`) of the meal date range.
    pub from: Option<String>,
    /// Inclusive upper bound (`YYYY-MM-DD`) of the meal date range.
    pub to: Option<String>,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of a `GroceryList`, returned by create/get/list.
#[derive(Debug, Serialize)]
pub struct GroceryListResponse {
    /// UUID v7 string (hyphenated) — the list's stable identifier.
    pub id: String,
    /// The list's display name, shown verbatim; never blank.
    pub name: String,
    /// Creation timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
}

/// JSON representation of a `GroceryItem`, returned by create/list/generate.
#[derive(Debug, Serialize)]
pub struct GroceryItemResponse {
    /// UUID v7 string (hyphenated) — the item's stable identifier.
    pub id: String,
    /// Which list this item belongs to.
    pub list_id: String,
    /// The item's name, shown verbatim; never blank.
    pub name: String,
    /// Freetext quantity; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
    /// Free-form category; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether the item has been checked off.
    pub checked: bool,
    /// `"manual"` or `"from_meal"`.
    pub source: String,
    /// UUID of the meal this item was generated from; omitted for manual
    /// items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_meal_id: Option<String>,
    /// Creation timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
}

/// JSON envelope returned by `POST /api/v1/grocery-lists/{id}/generate`.
///
/// See this module's top-level doc comment for why `added` carries only the
/// new items, not the full list.
#[derive(Debug, Serialize)]
pub struct GenerateResponse {
    /// The resolved inclusive lower bound of the meal date range used
    /// (`YYYY-MM-DD`) — echoed so the client can label the result even when
    /// it relied on the default-week fallback.
    pub from: String,
    /// The resolved inclusive upper bound of the meal date range used
    /// (`YYYY-MM-DD`).
    pub to: String,
    /// The newly-added items (may be empty — a quiet, correct outcome when
    /// every ingredient was already a pantry staple or already on the list).
    pub added: Vec<GroceryItemResponse>,
}

// ─── Handlers: lists ────────────────────────────────────────────────────────

/// `POST /api/v1/grocery-lists` — create a new grocery list.
///
/// Returns HTTP 422 for a blank name, 500 for an unexpected storage error.
pub async fn create_grocery_list(
    State(state): State<AppState>,
    Json(req): Json<CreateGroceryListRequest>,
) -> impl IntoResponse {
    // One `now`, read once at this I/O edge, anchors `created_at`.
    let now = OffsetDateTime::now_utc();

    // Validate the non-empty-name invariant and construct.
    let list = match GroceryListBuilder::new(req.name).now(now).build() {
        Ok(l) => l,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return grocery_error_response(&e),
    };

    // Persist the new list; a failure here is unexpected.
    if let Err(e) = insert_grocery_list(&state.db, &list).await {
        // Log the storage description; do not leak it to the client.
        tracing::error!(error = %e, "failed to insert grocery list");
        // A genuine storage failure, not a client mistake.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 Created with the full list so the client can display it without a GET.
    (StatusCode::CREATED, Json(grocery_list_to_response(&list))).into_response()
}

/// `GET /api/v1/grocery-lists` — list every grocery list.
///
/// Returns the whole set as a flat array (matching `list_events_handler`'s
/// bare-array convention).
///
/// Returns HTTP 200 with a JSON array (possibly empty), 500 on an unexpected
/// storage error.
pub async fn list_grocery_lists_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_grocery_lists(&state.db).await {
        // Success: convert each domain list to its flat JSON representation.
        Ok(lists) => {
            let responses: Vec<GroceryListResponse> =
                lists.iter().map(grocery_list_to_response).collect();
            Json(responses).into_response()
        }
        // Unexpected storage error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list grocery lists");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/grocery-lists/{id}` — fetch one list by UUID.
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the list
/// otherwise.
pub async fn get_grocery_list(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<GroceryListId>() else {
        return bad_request("invalid grocery list ID");
    };
    match fetch_grocery_list(&state.db, id).await {
        // Found: serialise and return 200.
        Ok(Some(list)) => Json(grocery_list_to_response(&list)).into_response(),
        // No row for that id: 404.
        Ok(None) => not_found("grocery list not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch grocery list");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Handlers: items ────────────────────────────────────────────────────────

/// `POST /api/v1/grocery-lists/{id}/items` — add an item manually.
///
/// The list must exist. The created item always carries `source: "manual"`
/// (`GroceryItemBuilder`'s own default) — meal-generated items only ever come
/// from `/generate`.
///
/// Returns 400 for a malformed list id, 404 if the list is missing, 422 for a
/// blank item name, 201 on success.
pub async fn create_grocery_item(
    State(state): State<AppState>,
    Path(list_id_str): Path<String>,
    Json(req): Json<CreateGroceryItemRequest>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(list_id) = list_id_str.parse::<GroceryListId>() else {
        return bad_request("invalid grocery list ID");
    };

    // The list must exist before an item can be filed under it — a 404 here
    // is clearer than a dangling item with no visible parent.
    match fetch_grocery_list(&state.db, list_id).await {
        // Exists: proceed.
        Ok(Some(_)) => {}
        // No such list → 404.
        Ok(None) => return not_found("grocery list not found"),
        // Storage failure → 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch grocery list for item add");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // One `now`, read once, anchors `created_at`.
    let now = OffsetDateTime::now_utc();

    // Build the item; source defaults to Manual, exactly what this endpoint
    // wants — it is never overridden here.
    let mut builder = GroceryItemBuilder::new(list_id, req.name).now(now);
    // Optional freetext quantity.
    if let Some(qty) = req.qty {
        builder = builder.qty(qty);
    }
    // Optional free-form category.
    if let Some(category) = req.category {
        builder = builder.category(category);
    }

    // Validate the non-empty-name invariant and construct.
    let item = match builder.build() {
        Ok(i) => i,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return grocery_error_response(&e),
    };

    // Persist the new item; a failure here is unexpected.
    if let Err(e) = insert_grocery_item(&state.db, &item).await {
        tracing::error!(error = %e, "failed to insert grocery item");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 Created with the full item so the client can display it without a GET.
    (StatusCode::CREATED, Json(grocery_item_to_response(&item))).into_response()
}

/// `GET /api/v1/grocery-lists/{id}/items` — list a list's items.
///
/// The list must exist (a 404 for an unknown list id, rather than a silently
/// empty array indistinguishable from "no items yet").
///
/// Returns 400 for a malformed id, 404 if the list is missing, 200 with a
/// JSON array (possibly empty) otherwise.
pub async fn list_grocery_items_handler(
    State(state): State<AppState>,
    Path(list_id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(list_id) = list_id_str.parse::<GroceryListId>() else {
        return bad_request("invalid grocery list ID");
    };

    // The list must exist — see this handler's doc comment for why a 404
    // beats a silently empty array.
    match fetch_grocery_list(&state.db, list_id).await {
        // Exists: proceed.
        Ok(Some(_)) => {}
        // No such list → 404.
        Ok(None) => return not_found("grocery list not found"),
        // Storage failure → 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch grocery list for item list");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    match list_grocery_items(&state.db, list_id).await {
        // Success: convert each domain item to its flat JSON representation.
        Ok(items) => {
            let responses: Vec<GroceryItemResponse> =
                items.iter().map(grocery_item_to_response).collect();
            Json(responses).into_response()
        }
        // Unexpected storage error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list grocery items");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `PATCH /api/v1/grocery-items/{id}` — toggle checked.
///
/// There is no `fetch_grocery_item(id)` in the storage layer (only a
/// per-list listing and a per-list fetch), so — unlike `patch_calendar`,
/// which re-reads the full row — this handler returns a small confirmation
/// body rather than the updated item, mirroring `create_override`'s and
/// `delete_calendar_handler`'s confirmation-only responses. A client that
/// wants the fresh item re-fetches the list.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn patch_grocery_item(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<PatchGroceryItemRequest>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<GroceryItemId>() else {
        return bad_request("invalid grocery item ID");
    };

    match set_grocery_item_checked(&state.db, id, req.checked).await {
        // Matched and updated: a small confirmation body (see this handler's
        // doc comment for why there is no full-item re-read).
        Ok(true) => (
            StatusCode::OK,
            Json(json!({ "id": id.to_string(), "checked": req.checked })),
        )
            .into_response(),
        // No such item: 404.
        Ok(false) => not_found("grocery item not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to update grocery item checked flag");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/v1/grocery-items/{id}` — remove an item.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn delete_grocery_item_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<GroceryItemId>() else {
        return bad_request("invalid grocery item ID");
    };
    match delete_grocery_item(&state.db, id).await {
        // Matched and removed: a small confirmation body.
        Ok(true) => (
            StatusCode::OK,
            Json(json!({ "id": id.to_string(), "deleted": true })),
        )
            .into_response(),
        // No such item: 404.
        Ok(false) => not_found("grocery item not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to delete grocery item");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Handler: generation ────────────────────────────────────────────────────

/// `POST /api/v1/grocery-lists/{id}/generate` — generate additions from
/// planned meals.
///
/// This is P2 Slice 1's `plan_grocery_additions` finally wired to storage:
/// loads the meals in `[from, to]`, every pantry staple, and the list's
/// current items, hands them to the pure function, persists whatever comes
/// back, and returns just that delta (see this module's top-level doc
/// comment for why the response is the delta, not the full list). The
/// no-clobber property is structural, not re-checked here — `existing` is
/// read fresh on every call and `plan_grocery_additions` only ever *adds*, so
/// a manually-checked or manually-added item that was already loaded into
/// `existing` can never be touched by this handler, no matter how many times
/// it runs.
///
/// `from`/`to` default to the current Monday-Sunday week (the same
/// computation `GET /api/v1/week` uses for its own default) when neither is
/// supplied; supplying only one of the pair is a 400.
///
/// Returns 400 for a malformed list id or date range, 404 if the list is
/// missing, 500 for an unexpected storage error, 200 with the generated
/// additions otherwise.
pub async fn generate_grocery_items(
    State(state): State<AppState>,
    Path(list_id_str): Path<String>,
    Query(params): Query<GenerateQuery>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(list_id) = list_id_str.parse::<GroceryListId>() else {
        return bad_request("invalid grocery list ID");
    };

    // The list must exist before anything can be generated onto it.
    match fetch_grocery_list(&state.db, list_id).await {
        // Exists: proceed.
        Ok(Some(_)) => {}
        // No such list → 404.
        Ok(None) => return not_found("grocery list not found"),
        // Storage failure → 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch grocery list for generate");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Capture `now` once — it anchors both the default-week fallback and the
    // generated items' `created_at`.
    let now = OffsetDateTime::now_utc();

    // Resolve the meal date range: an explicit (from, to) pair wins; absent
    // means the current Monday-Sunday week; half a pair is a 400.
    let (from, to) = match (params.from.as_deref(), params.to.as_deref()) {
        // Both present: parse each bound explicitly.
        (Some(from_str), Some(to_str)) => {
            // The inclusive lower bound.
            let from = match parse_date(from_str) {
                Ok(d) => d,
                // A malformed date is a 400, same treatment as a bad path id.
                Err(msg) => return bad_request(&msg),
            };
            // The inclusive upper bound.
            let to = match parse_date(to_str) {
                Ok(d) => d,
                Err(msg) => return bad_request(&msg),
            };
            (from, to)
        }
        // Neither present: fall back to the current Monday-Sunday week.
        (None, None) => current_week_bounds(now),
        // Exactly one of the pair — always a client error.
        _ => return bad_request("from and to must both be provided or both absent"),
    };

    // Load the three inputs `plan_grocery_additions` needs. A failure to read
    // any of them is unexpected — unlike surfacing's graceful degradation,
    // a partial generation run would be actively misleading (it might
    // silently omit a pantry-suppressed item because the pantry read failed),
    // so every read here is fatal to the request.
    // Input 1: every meal in the resolved range.
    let meals = match list_meals_in_range(&state.db, from, to).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "failed to list meals for grocery generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // Input 2: every pantry staple, so already-stocked ingredients are
    // suppressed.
    let pantry = match list_pantry_items(&state.db).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to list pantry items for grocery generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // Input 3: the list's current items, so the no-clobber rule has
    // something fresh to compare against (see this function's doc comment).
    let existing = match list_grocery_items(&state.db, list_id).await {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to list existing grocery items for generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // The pure function decides what's new; this handler only persists and
    // reports it. `existing` was just read fresh above, so a checked or
    // manually-added item is guaranteed to be in it — no-clobber falls out
    // of the pure function's own contract (see its doc comment).
    let additions = plan_grocery_additions(&meals, &pantry, &existing, list_id, now);

    // Persist the whole batch atomically; an empty `additions` still makes a
    // (trivial) call, kept simple rather than special-cased.
    if let Err(e) = insert_grocery_items(&state.db, &additions).await {
        // Log the storage description; do not leak it to the client.
        tracing::error!(error = %e, "failed to persist generated grocery items");
        // A genuine storage failure, not a client mistake.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 200 OK with just the delta (see this module's top-level doc comment).
    let body = GenerateResponse {
        // Echo the resolved range so the client can label the result even
        // when it relied on the default-week fallback.
        from: format_date(from),
        to: format_date(to),
        // The newly-persisted additions, reshaped for the wire.
        added: additions.iter().map(grocery_item_to_response).collect(),
    };
    Json(body).into_response()
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// The current Monday-Sunday week's `(start, end)` bounds, both inclusive.
///
/// Mirrors `api/surfacing.rs::week`'s own default-window computation exactly
/// (step back to the Monday by `now`'s weekday offset, then the Sunday six
/// days later) so "this week" means the same 7 days on both endpoints.
fn current_week_bounds(now: OffsetDateTime) -> (Date, Date) {
    // Just the calendar date; the time-of-day is irrelevant to the bounds.
    let today = now.date();
    // Step back by today's weekday offset from Monday (0 for Monday itself,
    // up to 6 for Sunday) to land on this week's Monday.
    let week_start = today - Duration::days(i64::from(today.weekday().number_days_from_monday()));
    // The week's Sunday, six days after its Monday.
    let week_end = week_start + Duration::days(6);
    // Both bounds, inclusive.
    (week_start, week_end)
}

/// Project a domain `GroceryList` into its JSON response shape.
fn grocery_list_to_response(list: &GroceryList) -> GroceryListResponse {
    GroceryListResponse {
        // UUID → string.
        id: list.id.to_string(),
        // Name verbatim.
        name: list.name.clone(),
        // Audit timestamp as an RFC 3339 string.
        created_at: list.created_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Project a domain `GroceryItem` into its JSON response shape.
fn grocery_item_to_response(item: &GroceryItem) -> GroceryItemResponse {
    GroceryItemResponse {
        // UUID → string.
        id: item.id.to_string(),
        // Which list this item belongs to.
        list_id: item.list_id.to_string(),
        // Name verbatim.
        name: item.name.clone(),
        // Freetext quantity, or omitted when absent.
        qty: item.qty.clone(),
        // Free-form category, or omitted when absent.
        category: item.category.clone(),
        // Checked-off flag, passed straight through.
        checked: item.checked,
        // Enum → its snake_case Display string ("manual" / "from_meal").
        source: item.source.to_string(),
        // Originating meal, or omitted for a manual item.
        source_meal_id: item.source_meal_id.map(|m| m.to_string()),
        // Audit timestamp as an RFC 3339 string.
        created_at: item.created_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Map a `GroceryError` to a 422 response.
///
/// Every builder error is a client-side validation failure, so both variants
/// (`EmptyName`, `MissingNow`) map to Unprocessable Entity with the domain
/// error's message; `MissingNow` cannot actually be reached from a handler in
/// this module since every builder call here sets `.now()` unconditionally,
/// but the mapping stays generic rather than special-casing it away.
fn grocery_error_response(e: &GroceryError) -> axum::response::Response {
    unprocessable(&format!("{e}"))
}

/// Parse a `YYYY-MM-DD` string into a `time::Date`.
///
/// Mirrors the date parsing in api/meal.rs and api/surfacing.rs so every
/// date-taking endpoint accepts the same format and produces the same style
/// of error message.
fn parse_date(s: &str) -> Result<Date, String> {
    // The bracketed format is a compile-time constant, so building it never fails.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant");
    // Quote the bad value in the error so it is easy to spot in logs.
    Date::parse(s, &fmt).map_err(|_| format!("date '{s}': expected YYYY-MM-DD"))
}

/// Format a `time::Date` as `YYYY-MM-DD` for the response body.
fn format_date(date: Date) -> String {
    // Same constant format as `parse_date`; formatting a valid date never fails.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant");
    date.format(&fmt)
        .expect("formatting a valid date never fails")
}

/// A 422 Unprocessable Entity response with an error message.
///
/// Used for every client-side validation failure (blank list/item name).
/// Mirrors the identically-named helper in `api/meal.rs` and `api/event.rs`.
fn unprocessable(message: &str) -> axum::response::Response {
    // 422 with a { "error": ... } body the client can display.
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": message })),
    )
        .into_response()
}

/// A 400 Bad Request response with an error message.
///
/// Used when a path parameter is malformed, or a `generate` date-range
/// parameter is invalid or half-supplied.
/// Mirrors the identically-named helper in `api/meal.rs` and `api/event.rs`.
fn bad_request(message: &str) -> axum::response::Response {
    // 400 with a JSON error body.
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

/// A 404 Not Found response with an error message.
///
/// Used when an id is well-formed but names no row.
/// Mirrors the identically-named helper in `api/meal.rs` and `api/event.rs`.
fn not_found(message: &str) -> axum::response::Response {
    // 404 with a JSON error body.
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}
