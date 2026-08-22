// api/pantry.rs — HTTP handlers for the Pantry API.
//
// Endpoints:
//   POST   /api/v1/pantry        — record a new staple
//   GET    /api/v1/pantry        — list every staple
//   DELETE /api/v1/pantry/{id}   — remove a staple
//
// `PantryItem` (amity_core::pantry) is the household's deliberately
// lightweight "staples memory" — a name and an optional note, nothing more
// (see its module doc for what is NOT modelled: stock levels, thresholds,
// purchase history). There is no PATCH: a staple's only mutable action is
// removal, and there is no `update_pantry_item` in the storage layer to
// support editing (mirrors `api/meal.rs`'s own no-PATCH rationale).
//
// Handler shape mirrors api/meal.rs and api/event.rs: `State(state):
// State<AppState>`, `Json(req): Json<...>` request bodies, `(StatusCode,
// Json(resp))` success responses, and the same `unprocessable`/`bad_request`/
// `not_found` helpers (kept local to this module, matching the existing
// per-module convention).
//
// Error mapping (matching api/meal.rs):
//   400 Bad Request    — a path parameter is not a valid UUID.
//   404 Not Found      — a valid id names no pantry item.
//   422 Unprocessable  — a body field is invalid (blank name).
//   500 Internal Error — an unexpected storage failure (logged, not leaked).

// axum plumbing.
// The JSON extractor/response wrapper used by every handler in this module.
use axum::Json;
// `Path` extracts `{id}`; `State` injects the shared `AppState`.
use axum::extract::{Path, State};
// HTTP status codes used in the response tuples below.
use axum::http::StatusCode;
// Sealed trait implemented by tuples, `Json<T>`, `StatusCode`, … for dispatch.
use axum::response::IntoResponse;
// Request/response (de)serialisation.
// `Deserialize` for request bodies; `Serialize` for response bodies.
use serde::{Deserialize, Serialize};
// Builds the small `{ "error": ... }` bodies for the 4xx helpers below.
use serde_json::json;
// OffsetDateTime for `created_at`; Rfc3339 for the wire.
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// Domain types.
use amity_core::ids::PantryItemId;
use amity_core::pantry::{PantryError, PantryItem, PantryItemBuilder};

// Storage.
use amity_storage::pantry::{delete_pantry_item, insert_pantry_item, list_pantry_items};

// Shared app state (the pool).
use crate::AppState;

// ─── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/pantry`.
///
/// Only `name` is required; must be non-empty after trimming.
#[derive(Debug, Deserialize)]
pub struct CreatePantryItemRequest {
    /// The staple's name (e.g. "flour"); matched case-insensitively against
    /// ingredient names by `grocery::plan_grocery_additions`.
    pub name: String,
    /// Optional free-form note (e.g. "keep two bags, we bake a lot").
    pub note: Option<String>,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of a `PantryItem`, returned by create/list.
#[derive(Debug, Serialize)]
pub struct PantryItemResponse {
    /// UUID v7 string (hyphenated) — the item's stable identifier.
    pub id: String,
    /// The staple's name, shown verbatim; never blank.
    pub name: String,
    /// Free-form note; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Creation timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `POST /api/v1/pantry` — record a new staple.
///
/// Builds and validates the domain `PantryItem` (non-empty name), persists
/// it, and returns 201 with the created item.
///
/// Returns HTTP 422 for a blank name, 500 for an unexpected storage error.
pub async fn create_pantry_item(
    State(state): State<AppState>,
    Json(req): Json<CreatePantryItemRequest>,
) -> impl IntoResponse {
    // One `now`, read once at this I/O edge, anchors `created_at`.
    let now = OffsetDateTime::now_utc();

    // Start the builder with the required name.
    let mut builder = PantryItemBuilder::new(req.name).now(now);
    // Optional free-form note.
    if let Some(note) = req.note {
        builder = builder.note(note);
    }

    // Validate all invariants (non-empty name, now) and construct.
    let item = match builder.build() {
        Ok(i) => i,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return pantry_error_response(&e),
    };

    // Persist the new staple; a failure here is unexpected.
    if let Err(e) = insert_pantry_item(&state.db, &item).await {
        tracing::error!(error = %e, "failed to insert pantry item");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 Created with the full item so the client can display it without a GET.
    (StatusCode::CREATED, Json(pantry_item_to_response(&item))).into_response()
}

/// `GET /api/v1/pantry` — list every recorded staple.
///
/// Returns the whole set as a flat array (matching `list_events_handler`'s
/// bare-array convention) — the pantry list is small and has no filters.
///
/// Returns HTTP 200 with a JSON array (possibly empty), 500 on an unexpected
/// storage error.
pub async fn list_pantry_items_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_pantry_items(&state.db).await {
        Ok(items) => {
            let responses: Vec<PantryItemResponse> =
                items.iter().map(pantry_item_to_response).collect();
            Json(responses).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list pantry items");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/v1/pantry/{id}` — remove a staple.
///
/// There is no confirmation step; the client is expected to confirm with the
/// user before calling this, mirroring `delete_calendar_handler`.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn delete_pantry_item_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<PantryItemId>() else {
        return bad_request("invalid pantry item ID");
    };
    match delete_pantry_item(&state.db, id).await {
        // Matched and removed: a small confirmation body.
        Ok(true) => (
            StatusCode::OK,
            Json(json!({ "id": id.to_string(), "deleted": true })),
        )
            .into_response(),
        Ok(false) => not_found("pantry item not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete pantry item");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Project a domain `PantryItem` into its JSON response shape.
fn pantry_item_to_response(item: &PantryItem) -> PantryItemResponse {
    PantryItemResponse {
        // UUID → string.
        id: item.id.to_string(),
        // Name verbatim.
        name: item.name.clone(),
        // Passthrough optional note.
        note: item.note.clone(),
        // Audit timestamp as an RFC 3339 string.
        created_at: item.created_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Map a `PantryError` to a 422 response.
///
/// Every builder error is a client-side validation failure, so both variants
/// map to Unprocessable Entity with the domain error's message.
fn pantry_error_response(e: &PantryError) -> axum::response::Response {
    unprocessable(&format!("{e}"))
}

/// A 422 Unprocessable Entity response with an error message.
///
/// Used for the sole client-side validation failure this module has: a
/// blank name.
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
/// Used when the path parameter is not a valid UUID.
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
