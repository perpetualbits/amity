// api/calendar.rs — HTTP handlers for the Calendars API.
//
// Endpoints:
//   POST   /api/v1/calendars              — subscribe to a new ICS feed
//   GET    /api/v1/calendars              — list all subscribed calendars
//   GET    /api/v1/calendars/{id}         — fetch one calendar (+ sync state)
//   PATCH  /api/v1/calendars/{id}         — toggle whether it is enabled
//   DELETE /api/v1/calendars/{id}         — unsubscribe (cascades its events)
//   POST   /api/v1/calendars/{id}/refresh — sync this one feed right now
//
// A `Calendar` (amity_core::calendar) is a subscribed read-only external ICS
// feed — school, club, waste, holiday, personal (brief §7). This module only
// manages the *subscription*; the actual fetch-parse-ingest pipeline lives in
// `jobs::calendar_sync`, which `refresh_calendar` calls directly for an
// on-demand sync. The background job (`jobs::calendar_sync::spawn`) covers
// the periodic 6-hourly case, so most calendars are never manually refreshed.
//
// Handler shape mirrors api/event.rs: `State(state): State<AppState>`,
// `Json(req): Json<...>` request bodies, `(StatusCode, Json(resp))` success
// responses, and the same `unprocessable`/`bad_request`/`not_found` helpers
// (kept local to this module rather than shared, matching the existing
// per-module convention — see api/event.rs's own copies).
//
// Error mapping (matching api/event.rs):
//   400 Bad Request       — a path parameter is not a valid UUID.
//   404 Not Found         — a valid id names no calendar.
//   422 Unprocessable     — a body field is invalid (blank name, non-http(s)
//                           URL scheme, unrecognised category).
//   500 Internal Error    — an unexpected storage or sync failure (logged,
//                           not leaked to the client).
//
// How an ingested event reaches Today: this module never touches events or
// surfacing directly. `refresh_calendar` (and the background job) upsert
// `Event` rows keyed by (calendar, UID) via `jobs::calendar_sync::sync_one`,
// which is the same mechanism `api/event.rs::create_event` feeds into for
// native events — so an ingested external event surfaces exactly like a
// native one, with no extra wiring here (see tests/calendar_api.rs's e2e).

// axum plumbing.
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
// Request/response (de)serialisation.
use serde::{Deserialize, Serialize};
use serde_json::json;
// Datetimes: OffsetDateTime for instants, Rfc3339 for the wire.
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// Domain types.
use amity_core::calendar::{Calendar, CalendarBuilder, CalendarSyncState};
use amity_core::ids::CalendarId;

// Storage: the calendars repository.
use amity_storage::calendar::{
    StoredCalendar, delete_calendar, fetch_calendar, insert_calendar, list_calendars,
    set_calendar_enabled,
};

// The sync job's single-calendar entry point, used by `refresh_calendar` for
// an on-demand sync; `feeds::fetch` is the real (network-touching) fetch
// function it is invoked with.
use crate::feeds;
use crate::jobs::calendar_sync::sync_one;

// Shared app state (the pool).
use crate::AppState;

// ─── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/calendars`.
///
/// Only `name` and `url` are required; `category` defaults to `Other` when
/// absent (matching `CalendarBuilder`'s own default). The URL may use
/// `webcal://` (rewritten to `https://`) or `http(s)://`; any other scheme is
/// a 422 — see `amity_core::calendar::normalise_feed_url`.
#[derive(Debug, Deserialize)]
pub struct CreateCalendarRequest {
    /// Human-facing display name; must be non-empty after trimming.
    pub name: String,
    /// The feed URL (webcal/http/https); normalised and scheme-checked by the
    /// builder.
    pub url: String,
    /// Advisory category string (e.g. `"school"`, `"waste"`); absent → Other.
    /// An unrecognised value is a 422, same as any other builder error.
    pub category: Option<String>,
}

/// Request body for `PATCH /api/v1/calendars/{id}`.
///
/// The only mutable field exposed here is `enabled` — toggling whether the
/// sync job fetches this feed. Renaming or re-pointing a feed's URL is not
/// supported (delete and re-subscribe instead), matching the brief's scope.
#[derive(Debug, Deserialize)]
pub struct PatchCalendarRequest {
    /// Whether the sync job should fetch this feed going forward.
    pub enabled: bool,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of a `Calendar` plus its current sync health, returned
/// by every handler in this module.
///
/// The two are separate domain types (a stable description vs. mutable
/// runtime status — see `amity_core::calendar`) but are flattened into one
/// response object here since API clients always want both together (there is
/// no use case for the subscription without its sync status, or vice versa).
#[derive(Debug, Serialize)]
pub struct CalendarResponse {
    /// UUID v7 string (hyphenated) — the calendar's stable identifier.
    pub id: String,
    /// Human-facing display name, shown verbatim.
    pub name: String,
    /// The feed URL, already normalised to `http(s)`.
    pub url: String,
    /// Advisory category string (`"school"`, `"waste"`, …).
    pub category: String,
    /// Whether the sync job currently fetches this feed.
    pub enabled: bool,
    /// Subscription creation timestamp (RFC 3339).
    pub created_at: String,
    /// The most recent sync attempt's outcome (`"never"`, `"ok"`,
    /// `"unreachable"`, `"parse_error"`).
    pub last_status: String,
    /// When the last *successful* sync completed; omitted until the first
    /// success (matching `EventResponse`'s convention of omitting absent
    /// optional fields rather than sending `null`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    /// A short diagnostic from the last failed attempt; omitted when the last
    /// attempt succeeded (or none has run yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// How many events the last successful sync produced.
    pub event_count: u32,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `POST /api/v1/calendars` — subscribe to a new external ICS feed.
///
/// Builds and validates the domain `Calendar` (non-empty name, `http(s)` URL,
/// a recognised category), persists it with fresh (never-synced) sync state,
/// and returns 201. The feed itself is not fetched here — the background sync
/// job picks it up on its next pass, or the caller can hit `/refresh`
/// immediately for an on-demand sync.
///
/// Returns 422 for any validation failure, 500 for an unexpected storage
/// error.
pub async fn create_calendar(
    State(state): State<AppState>,
    Json(req): Json<CreateCalendarRequest>,
) -> impl IntoResponse {
    // One `now` anchors both the builder's `created_at` and (implicitly) the
    // fresh `CalendarSyncState` returned below.
    let now = OffsetDateTime::now_utc();

    // Start the builder with the two required fields.
    let mut builder = CalendarBuilder::new(req.name, req.url).now(now);

    // Parse the optional category string, rejecting an unrecognised value
    // before it ever reaches the builder — same 422 treatment as a bad name
    // or URL.
    if let Some(category_str) = req.category {
        match category_str.parse() {
            Ok(category) => builder = builder.category(category),
            Err(e) => return unprocessable(&format!("category: {e}")),
        }
    }

    // Validate all invariants (name, URL scheme) and construct.
    let calendar = match builder.build() {
        Ok(c) => c,
        // Every builder error is a client-side validation failure → 422.
        Err(e) => return unprocessable(&format!("{e}")),
    };

    // Persist the subscription; a failure here is unexpected.
    if let Err(e) = insert_calendar(&state.db, &calendar).await {
        tracing::error!(error = %e, "failed to insert calendar");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // A freshly inserted calendar always has default (never-synced) sync
    // state — no extra read needed to build the response.
    let response = calendar_to_response(&calendar, &CalendarSyncState::default());
    (StatusCode::CREATED, Json(response)).into_response()
}

/// `GET /api/v1/calendars` — list every subscribed calendar.
///
/// Returns the whole set wrapped in a `{ "calendars": [...] }` envelope
/// (unlike the events list, which returns a bare array) — an explicit key
/// leaves room for pagination metadata alongside it later without a
/// breaking response-shape change.
///
/// Returns HTTP 200 with the envelope (an empty array when none exist).
pub async fn list_calendars_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_calendars(&state.db).await {
        // Success: wrap the converted rows in the envelope.
        Ok(rows) => {
            let calendars: Vec<CalendarResponse> = rows.iter().map(stored_to_response).collect();
            Json(json!({ "calendars": calendars })).into_response()
        }
        // Unexpected storage error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list calendars");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/calendars/{id}` — fetch one calendar by UUID.
///
/// The id is validated before the database is touched, so a typo is a cheap
/// 400 rather than a round-trip.
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the calendar
/// otherwise.
pub async fn get_calendar(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<CalendarId>() else {
        return bad_request("invalid calendar ID");
    };
    match fetch_calendar(&state.db, id).await {
        // Found: serialise and return 200.
        Ok(Some(stored)) => Json(stored_to_response(&stored)).into_response(),
        // No row for that id: 404.
        Ok(None) => not_found("calendar not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch calendar");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `PATCH /api/v1/calendars/{id}` — enable or disable a subscription.
///
/// Toggles whether the background sync job fetches this feed, without
/// touching its stored sync state or events (disabling archives a feed
/// without deleting its already-ingested data — see
/// `amity_core::calendar::CalendarBuilder::enabled`'s doc comment).
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the updated
/// calendar otherwise.
pub async fn patch_calendar(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<PatchCalendarRequest>,
) -> impl IntoResponse {
    let Ok(id) = id_str.parse::<CalendarId>() else {
        return bad_request("invalid calendar ID");
    };

    // Apply the toggle; `false` means the id matched no row.
    match set_calendar_enabled(&state.db, id, req.enabled).await {
        Ok(true) => {}
        Ok(false) => return not_found("calendar not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to update calendar enabled flag");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Re-read the row so the response reflects the persisted state rather
    // than assembling one by hand from the request.
    match fetch_calendar(&state.db, id).await {
        Ok(Some(stored)) => Json(stored_to_response(&stored)).into_response(),
        // The row vanished between the update and the re-read (a concurrent
        // delete) — treat it the same as "not found".
        Ok(None) => not_found("calendar not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to re-fetch calendar after patch");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/v1/calendars/{id}` — unsubscribe from a feed.
///
/// Cascades to every event (and event instance) the calendar owns — see
/// `amity_storage::calendar::delete_calendar`'s doc comment for the
/// dependency-ordered deletes this triggers. There is no confirmation step;
/// the client is expected to confirm with the user before calling this.
///
/// Returns 400 for a malformed id, 404 if not found, 200 on success.
pub async fn delete_calendar_handler(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    let Ok(id) = id_str.parse::<CalendarId>() else {
        return bad_request("invalid calendar ID");
    };
    match delete_calendar(&state.db, id).await {
        // Matched and removed: a small confirmation body, mirroring the
        // override-creation confirmation in api/event.rs.
        Ok(true) => (
            StatusCode::OK,
            Json(json!({ "id": id.to_string(), "deleted": true })),
        )
            .into_response(),
        // No such calendar: 404.
        Ok(false) => not_found("calendar not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete calendar");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `POST /api/v1/calendars/{id}/refresh` — sync this one feed right now.
///
/// Loads the calendar, then calls `jobs::calendar_sync::sync_one` with the
/// real network-touching `feeds::fetch` — the same function the background
/// job calls, so a manual refresh and the periodic 6-hourly one behave
/// identically. `now` is read once here, at the handler edge: core stays
/// clock-free, but a handler is I/O by nature and is the conventional place
/// to read the wall clock (matching `create_event`'s own `now` capture).
///
/// A fetch/parse failure is not an error response — it is a normal outcome
/// recorded on the calendar's sync state (`Unreachable`/`ParseError`; see
/// `sync_one`'s doc comment), so this handler still returns 200 with the
/// refreshed row showing that status. Only a genuine storage failure is a
/// 500.
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the refreshed
/// calendar otherwise.
pub async fn refresh_calendar(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    let Ok(id) = id_str.parse::<CalendarId>() else {
        return bad_request("invalid calendar ID");
    };

    // Load the calendar (and its current sync state) to hand to `sync_one`.
    let stored = match fetch_calendar(&state.db, id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found("calendar not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch calendar for refresh");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Read the clock once, at this I/O edge, for both the sync's `now` and
    // (implicitly) its recorded `last_synced_at` on success.
    let now = OffsetDateTime::now_utc();

    // Run the real sync pipeline for this one calendar. Only a storage-level
    // failure surfaces as `Err` here — a fetch/parse problem is already
    // folded into the calendar's own sync state by `sync_one`.
    if let Err(e) = sync_one(&state.db, now, &stored, &|url| feeds::fetch(url)).await {
        tracing::error!(error = %e, calendar_id = %id, "calendar refresh failed");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Re-read the row so the response reflects whatever `sync_one` just
    // recorded (Ok/Unreachable/ParseError, updated event_count, …).
    match fetch_calendar(&state.db, id).await {
        Ok(Some(refreshed)) => Json(stored_to_response(&refreshed)).into_response(),
        // Deleted mid-refresh — an edge case, still a coherent 404.
        Ok(None) => not_found("calendar not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to re-fetch calendar after refresh");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Project a `StoredCalendar` (the repository's combined read model) into its
/// JSON response shape.
fn stored_to_response(stored: &StoredCalendar) -> CalendarResponse {
    calendar_to_response(&stored.calendar, &stored.sync)
}

/// Project a `Calendar` and its `CalendarSyncState` into the flat JSON
/// response shape. Split from `stored_to_response` so `create_calendar` can
/// call it directly with a freshly built (not-yet-`StoredCalendar`) pair.
fn calendar_to_response(calendar: &Calendar, sync: &CalendarSyncState) -> CalendarResponse {
    CalendarResponse {
        // UUID → string.
        id: calendar.id.to_string(),
        // Passthrough fields.
        name: calendar.name.clone(),
        url: calendar.url.clone(),
        // Stored enums → their snake_case Display strings.
        category: calendar.category.to_string(),
        enabled: calendar.enabled,
        // Format the required timestamp as RFC 3339.
        created_at: calendar.created_at.format(&Rfc3339).unwrap_or_default(),
        last_status: sync.last_status.to_string(),
        // Optional timestamp: format only when present.
        last_synced_at: sync
            .last_synced_at
            .map(|dt| dt.format(&Rfc3339).unwrap_or_default()),
        last_error: sync.last_error.clone(),
        event_count: sync.event_count,
    }
}

/// A 422 Unprocessable Entity response with an error message.
///
/// Used for every client-side validation failure (blank name, bad URL
/// scheme, unrecognised category).
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
/// Used when a path parameter is malformed (e.g. a non-UUID id).
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
