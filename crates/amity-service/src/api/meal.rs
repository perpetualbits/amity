// api/meal.rs — HTTP handlers for the Meal API.
//
// Endpoints:
//   POST   /api/v1/meals        — plan a new meal
//   GET    /api/v1/meals        — list meals (optionally ?from=&to= a date range)
//   GET    /api/v1/meals/{id}   — fetch one meal
//   DELETE /api/v1/meals/{id}   — remove a meal
//
// There is no PATCH here: `amity_storage::meal` has no `update_meal` (see its
// module doc — reconciling the `meal_ingredients` child rows on an edit needs
// its own design, so editing a planned meal is deferred to a follow-up rather
// than half-built in this slice). A household that wants to change a meal
// deletes and re-creates it for now.
//
// Handler shape mirrors api/event.rs and api/calendar.rs: `State(state):
// State<AppState>`, `Json(req): Json<...>` request bodies, `(StatusCode,
// Json(resp))` success responses, and the same `unprocessable`/`bad_request`/
// `not_found` helpers (kept local to this module, matching the existing
// per-module convention).
//
// Error mapping (matching api/event.rs and api/calendar.rs):
//   400 Bad Request   — a path parameter is not a valid UUID, or `?from=`/`?to=`
//                       is present but not YYYY-MM-DD (or only one is given).
//   404 Not Found     — a valid id names no meal.
//   422 Unprocessable — a body field is invalid (blank name, unknown slot,
//                       malformed cook id).
//   500 Internal Error — an unexpected storage failure (logged, not leaked).
//
// Field naming: the request/response bodies use `name` (not `title`) for the
// meal's headline field, matching `amity_core::meal::Meal::name` exactly —
// there is no separate wire vocabulary to translate here, unlike Event's
// `title`.

// axum plumbing.
// The JSON extractor/response wrapper used by every handler in this module.
use axum::Json;
// `Path` extracts `{id}`; `Query` extracts `?from=&to=`; `State` injects the
// shared `AppState`.
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
// Datetimes: OffsetDateTime for `created_at`; Rfc3339 for the wire; Date for
// the meal's calendar date and the `?from=`/`?to=` range bounds.
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime};

// Domain types.
// Typed ids for the path parameter and the optional cook reference.
use amity_core::ids::{MealId, MemberId};
// The Meal entity, its validating builder, its error type, and the slot enum.
use amity_core::meal::{IngredientLine, Meal, MealBuilder, MealError, MealSlot};

// Storage.
// The repository functions this module calls directly (see meal.rs's module
// doc for why there is no update_meal).
use amity_storage::meal::{delete_meal, fetch_meal, insert_meal, list_meals, list_meals_in_range};

// Shared app state (the pool).
use crate::AppState;

// ─── Request types ──────────────────────────────────────────────────────────

/// One ingredient line as sent by the client — mirrors
/// `amity_core::meal::IngredientLine` field for field.
#[derive(Debug, Deserialize)]
pub struct IngredientLineRequest {
    /// The ingredient's name (e.g. "flour"). Empty lines are not rejected
    /// here — an empty ingredient name is silently dropped by the builder's
    /// `.ingredient()` call producing a stored line with an empty name only
    /// if the caller insists; this mirrors the domain type's own looseness
    /// (only the meal's own `name` is validated non-empty).
    pub name: String,
    /// Freetext quantity, if the household chose to record one.
    pub qty: Option<String>,
}

/// Request body for `POST /api/v1/meals`.
///
/// Only `name` and `date` are required. `slot` defaults to `dinner`
/// (`MealBuilder`'s own default) when absent.
#[derive(Debug, Deserialize)]
pub struct CreateMealRequest {
    /// The meal's name (e.g. "Spaghetti bolognese"); must be non-empty after
    /// trimming.
    pub name: String,
    /// The calendar date this meal is planned for (`YYYY-MM-DD`).
    pub date: String,
    /// Which slot of the day: `"dinner"` (default) | `"breakfast"` | `"lunch"`
    /// | `"other"`. An unrecognised value is a 422.
    pub slot: Option<String>,
    /// UUID of the member cooking, if assigned. A malformed UUID is a 422.
    pub cook: Option<String>,
    /// Freetext ingredient lines; absent means none recorded yet.
    pub ingredient_lines: Option<Vec<IngredientLineRequest>>,
    /// Optional free-form notes (e.g. "double the sauce for leftovers").
    pub notes: Option<String>,
}

/// Query parameters for `GET /api/v1/meals`.
///
/// `from`/`to` must both be present (an inclusive date range) or both absent
/// (list every meal) — half a pair is a 400, matching the recurrence-pair
/// validation style used elsewhere in the service layer.
#[derive(Debug, Deserialize)]
pub struct ListMealsQuery {
    /// Inclusive lower bound (`YYYY-MM-DD`) of the date range.
    pub from: Option<String>,
    /// Inclusive upper bound (`YYYY-MM-DD`) of the date range.
    pub to: Option<String>,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of one ingredient line.
#[derive(Debug, Serialize)]
pub struct IngredientLineResponse {
    /// The ingredient's name, shown verbatim.
    pub name: String,
    /// Freetext quantity; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
}

/// JSON representation of a Meal, returned by create/get/list.
///
/// Optional fields are omitted from the body entirely when absent (clients
/// check key presence, not `null`), matching `EventResponse`'s convention.
#[derive(Debug, Serialize)]
pub struct MealResponse {
    /// UUID v7 string (hyphenated) — the meal's stable identifier.
    pub id: String,
    /// The meal's calendar date (`YYYY-MM-DD`).
    pub date: String,
    /// Which slot of the day (`"dinner"`, `"breakfast"`, `"lunch"`, `"other"`).
    pub slot: String,
    /// The meal's name, shown verbatim; never blank.
    pub name: String,
    /// UUID of the cook, if assigned; omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cook: Option<String>,
    /// Freetext ingredient lines, in the order they were recorded.
    pub ingredient_lines: Vec<IngredientLineResponse>,
    /// Free-form notes; omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Creation timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `POST /api/v1/meals` — plan a new meal.
///
/// Flow: parse the required date, parse the optional slot/cook (each
/// independently, so a bad value names itself in the 422), build and
/// validate the domain `Meal`, persist it, and return 201 with the created
/// meal.
///
/// Returns HTTP 422 for any validation failure, 500 for an unexpected
/// storage error.
pub async fn create_meal(
    State(state): State<AppState>,
    Json(req): Json<CreateMealRequest>,
) -> impl IntoResponse {
    // One `now` anchors `created_at`, read once at this I/O edge (core stays
    // clock-free), mirroring `create_event`'s own capture.
    let now = OffsetDateTime::now_utc();

    // Parse the required calendar date before touching the builder.
    let date = match parse_date(&req.date) {
        Ok(d) => d,
        Err(msg) => return unprocessable(&msg),
    };

    // Start the builder with the two required fields.
    let mut builder = MealBuilder::new(req.name).date(date).now(now);

    // Parse the optional slot, rejecting an unrecognised value before it ever
    // reaches the builder — same 422 treatment as a bad name or date.
    if let Some(slot_str) = req.slot {
        match slot_str.parse::<MealSlot>() {
            Ok(slot) => builder = builder.slot(slot),
            Err(e) => return unprocessable(&format!("slot: {e}")),
        }
    }

    // Parse the optional cook reference; a malformed UUID is a 422, not a
    // silent drop.
    if let Some(cook_str) = req.cook {
        match cook_str.parse::<MemberId>() {
            Ok(cook) => builder = builder.cook(cook),
            Err(_) => return unprocessable("cook: invalid member ID"),
        }
    }

    // Apply each ingredient line, verbatim (no unit parsing — see meal.rs).
    if let Some(lines) = req.ingredient_lines {
        for line in lines {
            builder = builder.ingredient(line.name, line.qty);
        }
    }

    // Optional free-form notes.
    if let Some(notes) = req.notes {
        builder = builder.notes(notes);
    }

    // Validate all invariants (non-empty name, date, now) and construct.
    let meal = match builder.build() {
        Ok(m) => m,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return meal_error_response(&e),
    };

    // Persist the meal row (plus its ingredient lines, in one transaction);
    // a failure here is unexpected.
    if let Err(e) = insert_meal(&state.db, &meal).await {
        // Log the storage description; do not leak it to the client.
        tracing::error!(error = %e, "failed to insert meal");
        // A genuine storage failure, not a client mistake.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 Created with the full meal so the client can display it without a GET.
    (StatusCode::CREATED, Json(meal_to_response(&meal))).into_response()
}

/// `GET /api/v1/meals` — list meals, optionally filtered to a date range.
///
/// When both `?from=` and `?to=` are supplied, delegates to
/// `list_meals_in_range` (the grocery generator's own input path — exercised
/// here too, so the two share exactly one query shape). When neither is
/// supplied, lists every meal. Supplying only one of the pair is a 400.
///
/// Returns HTTP 200 with a JSON array (possibly empty), 400 for a malformed
/// or half-supplied range, 500 for an unexpected storage error.
pub async fn list_meals_handler(
    State(state): State<AppState>,
    Query(params): Query<ListMealsQuery>,
) -> impl IntoResponse {
    // Resolve which storage call to make from the (from, to) pair's shape.
    let result = match (params.from.as_deref(), params.to.as_deref()) {
        // Both present: parse each bound and delegate to the ranged query.
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
            // Delegate to the grocery generator's own storage query.
            list_meals_in_range(&state.db, from, to).await
        }
        // Neither present: the whole table.
        (None, None) => list_meals(&state.db).await,
        // Exactly one of the pair — always a client error, mirroring the
        // recurrence-pair validation in create_event.
        _ => return bad_request("from and to must both be provided or both absent"),
    };

    match result {
        // Success: return the array directly, no envelope (matching
        // list_events_handler's bare-array convention).
        Ok(meals) => {
            let responses: Vec<MealResponse> = meals.iter().map(meal_to_response).collect();
            Json(responses).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list meals");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/meals/{id}` — fetch one meal by UUID.
///
/// The id is validated before the database is touched, so a typo is a cheap
/// 400 rather than a round-trip.
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the meal
/// otherwise.
pub async fn get_meal(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<MealId>() else {
        return bad_request("invalid meal ID");
    };
    match fetch_meal(&state.db, id).await {
        // Found: serialise and return 200.
        Ok(Some(meal)) => Json(meal_to_response(&meal)).into_response(),
        // No row for that id: 404.
        Ok(None) => not_found("meal not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch meal");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/v1/meals/{id}` — remove a planned meal.
///
/// Cascades to its `meal_ingredients` rows (see
/// `amity_storage::meal::delete_meal`'s doc comment). There is no
/// confirmation step; the client is expected to confirm with the user before
/// calling this, mirroring `delete_calendar_handler`.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn delete_meal_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<MealId>() else {
        return bad_request("invalid meal ID");
    };
    match delete_meal(&state.db, id).await {
        // Matched and removed: a small confirmation body, mirroring
        // delete_calendar_handler.
        Ok(true) => (
            StatusCode::OK,
            Json(json!({ "id": id.to_string(), "deleted": true })),
        )
            .into_response(),
        // No such meal: 404.
        Ok(false) => not_found("meal not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to delete meal");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Project a domain `Meal` into its JSON response shape.
fn meal_to_response(meal: &Meal) -> MealResponse {
    MealResponse {
        // UUID → string.
        id: meal.id.to_string(),
        // Calendar date; formatting a valid date never fails.
        date: format_date(meal.date),
        // Enum → its snake_case Display string.
        slot: meal.slot.to_string(),
        // Name verbatim.
        name: meal.name.clone(),
        // Optional cook reference → string, or omitted.
        cook: meal.cook.map(|c| c.to_string()),
        // Ingredient lines, in stored order.
        ingredient_lines: meal
            .ingredient_lines
            .iter()
            .map(ingredient_line_to_response)
            .collect(),
        // Passthrough optional notes.
        notes: meal.notes.clone(),
        // Audit timestamp as an RFC 3339 string.
        created_at: meal.created_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Project one `IngredientLine` into its JSON response shape.
fn ingredient_line_to_response(line: &IngredientLine) -> IngredientLineResponse {
    IngredientLineResponse {
        // Name verbatim.
        name: line.name.clone(),
        // Freetext quantity, or omitted when absent.
        qty: line.qty.clone(),
    }
}

/// Map a `MealError` to a 422 response.
///
/// Every builder error is a client-side validation failure, so they all map
/// to Unprocessable Entity with the domain error's message.
fn meal_error_response(e: &MealError) -> axum::response::Response {
    unprocessable(&format!("{e}"))
}

/// Parse a `YYYY-MM-DD` string into a `time::Date`.
///
/// Mirrors the date parsing in api/event.rs and api/surfacing.rs so every
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
/// Used for every client-side validation failure (blank name, unknown slot,
/// malformed cook id).
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
/// Used when a path parameter is malformed, or a query parameter (a date, or
/// half of the `from`/`to` pair) is invalid.
fn bad_request(message: &str) -> axum::response::Response {
    // 400 with a JSON error body.
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

/// A 404 Not Found response with an error message.
///
/// Used when an id is well-formed but names no row.
fn not_found(message: &str) -> axum::response::Response {
    // 404 with a JSON error body.
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}
