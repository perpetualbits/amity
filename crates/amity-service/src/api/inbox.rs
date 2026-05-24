// api/inbox.rs — HTTP handlers for the Inbox API.
//
// Two endpoints:
//   POST /api/v1/inbox
//     Accepts: { "raw_text": "...", "source": "touch" }
//     Returns: the created InboxItem as JSON.
//
//   GET /api/v1/inbox/recent?limit=N
//     Returns: array of the N most recent InboxItems, newest first.
//     Default limit: 20. Maximum: 100.
//
// Both handlers delegate all domain logic to amity-core and all persistence
// to amity-storage. The handlers themselves only: parse the request, call the
// right functions, and serialise the response.
//
// The placeholder member ID (from migration 0001) is used for `captured_by`
// until the Member entity is implemented. This is a documented shortcut.

// Axum types: Json for request/response bodies, Query for query params,
// State for shared application state.
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
// IntoResponse allows returning different concrete response types from handlers.
use axum::response::IntoResponse;
// Serde: Deserialize for request body/query params, Serialize for response body.
use serde::{Deserialize, Serialize};
// OffsetDateTime is the timestamp type for `captured_at`.
use time::OffsetDateTime;

// Domain types: MemberId for the placeholder member, InboxItemBuilder to build
// validated items, InboxSource to model the capture mechanism.
use amity_core::ids::MemberId;
use amity_core::inbox::{InboxItemBuilder, InboxSource};
// Repository functions — the only storage-layer functions the handler touches.
use amity_storage::inbox::{insert_inbox_item, list_recent_inbox_items};

// AppState holds the SqlitePool shared across all request handlers.
use crate::AppState;

// ─── Request / response types ─────────────────────────────────────────────────

/// Request body for `POST /api/v1/inbox`.
///
/// Only two fields are required at capture time — the system supplies
/// `id`, `captured_by`, `captured_at`, and `triage_state` automatically.
// Deserialize: axum's Json extractor uses serde to parse the request body.
// Debug: enables test assertions on extracted request values.
#[derive(Debug, Deserialize)]
pub struct CaptureInboxItemRequest {
    /// The raw text of the captured thought. Must not be blank.
    pub raw_text: String,

    /// How the item was captured. Defaults to `touch` if absent.
    #[serde(default = "default_source")]
    pub source: InboxSource,
}

// Called by serde when `source` is absent from the request JSON.
// A named function is required by serde's `default = "fn_name"` attribute.
fn default_source() -> InboxSource {
    // Hub touch is the default capture path.
    InboxSource::Touch
}

/// Query parameters for `GET /api/v1/inbox/recent`.
// Deserialize: axum's Query extractor uses serde to parse the query string.
#[derive(Debug, Deserialize)]
pub struct ListRecentQuery {
    /// Maximum number of items to return. Capped at 100; defaults to 20.
    #[serde(default = "default_limit")]
    pub limit: u32,
}

// Called by serde when `limit` is absent from the query string.
// 20 is the inbox page size that fits on the hub's touch-optimised layout.
fn default_limit() -> u32 {
    20
}

// ─── Shared response type ─────────────────────────────────────────────────────

/// JSON representation of an inbox item, used in both the create and list
/// responses. This is a flat serialisation of `InboxItem` suitable for the
/// API surface; it does not expose internal storage details.
// Serialize: axum's Json response uses serde to produce the JSON body.
#[derive(Debug, Serialize)]
pub struct InboxItemResponse {
    // UUID string, matching InboxItemId::to_string() — hyphenated format.
    pub id: String,
    // Verbatim captured text — never normalised on read.
    pub raw_text: String,
    // UUID string of the member who captured this item.
    pub captured_by: String,
    /// RFC 3339 timestamp, e.g. `"2026-05-25T10:00:00Z"`.
    pub captured_at: String,
    // snake_case capture source (e.g. "touch", "mobile") — see InboxSource.
    pub source: String,
    // snake_case triage lifecycle state — see TriageState.
    pub triage_state: String,
    // Omitted from JSON when None — avoids {"triaged_to": null} in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triaged_to: Option<String>,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// `POST /api/v1/inbox` — capture a new inbox item.
///
/// Generates a fresh ID and timestamp, inserts the item into the database,
/// and returns the created item as JSON with HTTP 201 Created.
///
/// Returns HTTP 422 if `raw_text` is blank.
/// Returns HTTP 500 on unexpected storage errors.
///
/// # Panics
///
/// Panics if the hardcoded placeholder member UUID is not valid. This cannot
/// happen in practice because the UUID is a compile-time constant; the panic
/// is documented here to satisfy the pedantic lint.
pub async fn capture_inbox_item(
    State(state): State<AppState>,
    Json(req): Json<CaptureInboxItemRequest>,
) -> impl IntoResponse {
    // Use the system clock for `now`. The clock is not injected in the handler
    // because handlers are async axum functions with fixed signatures; tests
    // that need clock control should test the domain logic directly.
    let now = OffsetDateTime::now_utc();

    // Use the placeholder member ID until the Member entity is implemented.
    // See migration 0001_initial.sql for the rationale.
    let placeholder_member = MemberId(
        uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
            .expect("hardcoded UUID is always valid"),
    );

    // Build and validate the domain object. The builder validates that raw_text
    // is non-empty; the match handles each failure mode with a specific HTTP status.
    let item = match InboxItemBuilder::new()
        .raw_text(req.raw_text)
        .captured_by(placeholder_member)
        .now(now)
        .source(req.source)
        .build()
    {
        // Validation passed — proceed with the constructed item.
        Ok(item) => item,
        Err(amity_core::inbox::InboxError::EmptyText) => {
            // 422 Unprocessable Entity — the request was well-formed JSON but
            // the business rule (non-empty text) was violated.
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "raw_text must not be empty" })),
            )
                .into_response();
        }
        // Any other InboxError is unexpected — the builder only emits EmptyText
        // or MissingField, and MissingField cannot occur here (all fields are set).
        Err(e) => {
            tracing::error!(error = %e, "unexpected error building inbox item");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Persist the validated item. If storage fails, log the error and return 500 —
    // the item was never written so no compensation is needed.
    if let Err(e) = insert_inbox_item(&state.db, &item).await {
        // Log with structured fields for monitoring and alerting systems.
        tracing::error!(error = %e, "failed to insert inbox item");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Convert the domain item to the API response shape before serialising.
    let response = inbox_item_to_response(&item);

    // 201 Created with the full item in the body so the client can display it
    // immediately without a separate fetch.
    (StatusCode::CREATED, Json(response)).into_response()
}

/// `GET /api/v1/inbox/recent?limit=N` — list recent inbox items.
///
/// Returns HTTP 200 with a JSON array of items, newest first.
/// Returns HTTP 400 if `limit` exceeds 100.
pub async fn list_recent(
    State(state): State<AppState>,
    Query(params): Query<ListRecentQuery>,
) -> impl IntoResponse {
    // Enforce the maximum limit at the API boundary. Returning 400 rather than
    // silently capping the value lets clients detect and fix misuse.
    if params.limit > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "limit must be ≤ 100" })),
        )
            .into_response();
    }

    // Fetch items from storage. The storage layer returns them newest-first
    // because the query uses ORDER BY captured_at DESC.
    match list_recent_inbox_items(&state.db, params.limit).await {
        // Success: map each domain item to its JSON-serialisable response shape.
        Ok(items) => {
            let responses: Vec<InboxItemResponse> =
                items.iter().map(inbox_item_to_response).collect();
            // HTTP 200 with the array body — no status wrapper needed for success.
            Json(responses).into_response()
        }
        // Any storage error here is unexpected — log it and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list inbox items");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Private helpers ─────────────────────────────────────────────────────────

/// Convert a domain `InboxItem` into its JSON response representation.
///
/// Uses `Display` implementations for enum variants (which produce the same
/// `snake_case` strings stored in the database), so the API and storage wire
/// formats stay consistent.
fn inbox_item_to_response(item: &amity_core::inbox::InboxItem) -> InboxItemResponse {
    // RFC 3339 timestamp format — unambiguous, sortable, human-readable.
    let captured_at = item
        .captured_at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| item.captured_at.to_string());

    // Construct the flat response. Display impls for IDs and enums ensure the
    // API format matches the storage format without duplicating the string logic.
    InboxItemResponse {
        // id: Display gives the canonical hyphenated UUID string.
        id: item.id.to_string(),
        // raw_text is cloned because InboxItem is borrowed, not consumed.
        raw_text: item.raw_text.clone(),
        // captured_by: Display gives the member UUID string.
        captured_by: item.captured_by.to_string(),
        captured_at,
        // source and triage_state: Display gives the snake_case enum strings.
        source: item.source.to_string(),
        triage_state: item.triage_state.to_string(),
        // Extract the inner string from TypedEntityRef, if present.
        triaged_to: item.triaged_to.as_ref().map(|r| r.0.clone()),
    }
}
