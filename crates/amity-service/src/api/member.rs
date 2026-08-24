// api/member.rs — HTTP handlers for the Member API.
//
// Endpoints:
//   POST   /api/v1/members        — register a new household member
//   GET    /api/v1/members        — list every member
//   GET    /api/v1/members/{id}   — fetch one member
//   DELETE /api/v1/members/{id}   — remove a member
//
// A `Member` (amity_core::member) is a DISPLAY REGISTRY ENTRY ONLY — see that
// module's doc for the hard boundary this API must not cross: no accounts,
// auth, roles, activity, or age/child-specific fields. This module only ever
// reads/writes {id, display_name, initial, color, created_at}.
//
// Handler shape mirrors api/pantry.rs and api/calendar.rs: `State(state):
// State<AppState>`, `Json(req): Json<...>` request bodies, `(StatusCode,
// Json(resp))` success responses, and the same `unprocessable`/`bad_request`/
// `not_found` helpers (kept local to this module, matching the existing
// per-module convention). The list endpoint uses the `{ "members": [...] }`
// envelope, matching `list_calendars_handler` rather than pantry's bare array.
//
// Error mapping (matching api/pantry.rs / api/calendar.rs):
//   400 Bad Request    — a path parameter is not a valid UUID.
//   404 Not Found      — a valid id names no member.
//   422 Unprocessable  — a body field is invalid (blank display_name,
//                        unrecognised color string).
//   500 Internal Error — an unexpected storage failure (logged, not leaked).
//
// No PATCH: this slice is create/list/get/delete only, matching the storage
// layer (there is no `update_member`).

// axum plumbing.
use axum::Json;
// `Path` extracts `{id}`; `State` injects the shared `AppState`.
use axum::extract::{Path, State};
// HTTP status codes used in the response tuples below.
use axum::http::StatusCode;
// Sealed trait implemented by tuples, `Json<T>`, `StatusCode`, … for dispatch.
use axum::response::IntoResponse;
// Request/response (de)serialisation.
use serde::{Deserialize, Serialize};
// Builds the small `{ "error": ... }` / envelope bodies below.
use serde_json::json;
// OffsetDateTime for `created_at`; Rfc3339 for the wire.
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// Domain types.
use amity_core::ids::MemberId;
use amity_core::member::{Member, MemberBuilder, MemberColor, MemberError};

// Storage.
use amity_storage::member::{delete_member, fetch_member, insert_member, list_members};

// Shared app state (the pool).
use crate::AppState;

// ─── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/members`.
///
/// Only `display_name` is required; must be non-empty after trimming.
/// `color`, if present, must be one of the known `MemberColor` strings.
#[derive(Debug, Deserialize)]
pub struct CreateMemberRequest {
    /// The name shown throughout the hub.
    pub display_name: String,
    /// Optional short label (e.g. a single letter) for compact display.
    pub initial: Option<String>,
    /// Optional accent colour, as its `snake_case` string (e.g. `"sage"`).
    pub color: Option<String>,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of a `Member`, returned by create/list/get.
#[derive(Debug, Serialize)]
pub struct MemberResponse {
    /// UUID v7 string (hyphenated) — the member's stable identifier.
    pub id: String,
    /// The member's display name; never blank.
    pub display_name: String,
    /// Optional short label; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    /// Optional accent colour (`snake_case` string); omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Registration timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `POST /api/v1/members` — register a new household member.
///
/// Builds and validates the domain `Member` (non-empty `display_name`, known
/// color string), persists it, and returns 201 with the created member.
///
/// Returns HTTP 422 for a blank `display_name` or an unrecognised color, 500
/// for an unexpected storage error.
pub async fn create_member(
    State(state): State<AppState>,
    Json(req): Json<CreateMemberRequest>,
) -> impl IntoResponse {
    // One `now`, read once at this I/O edge, anchors `created_at`. amity-core
    // stays clock-free — see MemberBuilder::now's doc.
    let now = OffsetDateTime::now_utc();

    // Start the builder with the required display name.
    let mut builder = MemberBuilder::new(req.display_name).now(now);
    // Optional short label.
    if let Some(initial) = req.initial {
        builder = builder.initial(initial);
    }
    // Optional colour: parse the wire string before handing it to the
    // builder, so a bad color string surfaces as the same 422 path as a
    // blank name rather than a separate error shape.
    if let Some(color_str) = req.color {
        let color = match color_str.parse::<MemberColor>() {
            Ok(c) => c,
            Err(e) => return member_error_response(&e),
        };
        builder = builder.color(color);
    }

    // Validate all invariants (non-empty display_name, now) and construct.
    let member = match builder.build() {
        Ok(m) => m,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return member_error_response(&e),
    };

    // Persist the new member; a failure here is unexpected.
    if let Err(e) = insert_member(&state.db, &member).await {
        tracing::error!(error = %e, "failed to insert member");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 Created with the full member so the client can display it without a GET.
    (StatusCode::CREATED, Json(member_to_response(&member))).into_response()
}

/// `GET /api/v1/members` — list every registered member.
///
/// Returns the whole set wrapped in a `{ "members": [...] }` envelope
/// (matching `list_calendars_handler`) — the roster is small and has no
/// filters in this slice.
///
/// Returns HTTP 200 with the envelope (possibly an empty array), 500 on an
/// unexpected storage error.
pub async fn list_members_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_members(&state.db).await {
        Ok(members) => {
            let responses: Vec<MemberResponse> = members.iter().map(member_to_response).collect();
            Json(json!({ "members": responses })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list members");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/members/{id}` — fetch one member by id.
///
/// The id is validated before the database is touched, so a typo is a cheap
/// 400 rather than a round-trip. Returns 404 if the id is well-formed but
/// names no row — this is the expected shape for a dangling cook/assignee
/// reference elsewhere in the schema (see the storage module doc); callers
/// that need to resolve such an id render a neutral placeholder rather than
/// treating this 404 as an error.
pub async fn get_member(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<MemberId>() else {
        return bad_request("invalid member ID");
    };
    match fetch_member(&state.db, id).await {
        // Found: serialise and return 200.
        Ok(Some(member)) => Json(member_to_response(&member)).into_response(),
        // No row for that id: 404.
        Ok(None) => not_found("member not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch member");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/v1/members/{id}` — remove a member.
///
/// There is no confirmation step; the client is expected to confirm with the
/// user before calling this, mirroring `delete_pantry_item_handler`. This
/// does not touch any Task/Meal rows referencing the deleted id — see the
/// storage layer's module doc for why dangling references are expected.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn delete_member_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<MemberId>() else {
        return bad_request("invalid member ID");
    };
    match delete_member(&state.db, id).await {
        // Matched and removed: a small confirmation body, not the full member
        // (it no longer exists to serialise).
        Ok(true) => (
            // 200, not 204 — the body carries the confirmation fields below.
            StatusCode::OK,
            // Echo the id plus a boolean flag the client can branch on.
            Json(json!({ "id": id.to_string(), "deleted": true })),
        )
            .into_response(),
        Ok(false) => not_found("member not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete member");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Project a domain `Member` into its JSON response shape.
fn member_to_response(member: &Member) -> MemberResponse {
    // One field per wire field; see MemberResponse's own field docs above.
    MemberResponse {
        // UUID → string.
        id: member.id.to_string(),
        // Display name verbatim.
        display_name: member.display_name.clone(),
        // Passthrough optional initial.
        initial: member.initial.clone(),
        // Colour, if any, as its snake_case wire string.
        color: member.color.map(|c| c.to_string()),
        // Audit timestamp as an RFC 3339 string.
        created_at: member.created_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Map a `MemberError` to a 422 response.
///
/// Every builder/parse error here is a client-side validation failure, so
/// all variants map to Unprocessable Entity with the domain error's message.
fn member_error_response(e: &MemberError) -> axum::response::Response {
    unprocessable(&format!("{e}"))
}

/// A 422 Unprocessable Entity response with an error message.
fn unprocessable(message: &str) -> axum::response::Response {
    (
        // The one non-200 status this module returns for a validation failure.
        StatusCode::UNPROCESSABLE_ENTITY,
        // Same `{ "error": ... }` shape as bad_request/not_found below.
        Json(json!({ "error": message })),
    )
        .into_response()
}

/// A 400 Bad Request response with an error message.
///
/// Used when the path parameter is not a valid UUID.
fn bad_request(message: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

/// A 404 Not Found response with an error message.
///
/// Used when an id is well-formed but names no row.
fn not_found(message: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}
