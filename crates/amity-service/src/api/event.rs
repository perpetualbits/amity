// api/event.rs — HTTP handlers for the Event API.
//
// Endpoints:
//   POST /api/v1/events                 — create a native event (materialise
//                                         instances if recurring)
//   GET  /api/v1/events                 — list all events (by start time)
//   GET  /api/v1/events/{id}            — fetch one event
//   POST /api/v1/events/{id}/override   — create an EventOverride on an instance
//
// The handlers follow the same shape as api/task.rs: parse → validate domain
// rules → storage → serialise. Native events are created here; read-only
// external events arrive via ICS ingestion (Task 5), and their instances are
// edited only through the override endpoint.
//
// Placeholder member: the override's `created_by` uses the migration-0001
// placeholder until real member management lands.
//
// Error mapping (matching api/task.rs):
//   400 Bad Request       — a path parameter is not a valid UUID.
//   404 Not Found         — a valid id names no event.
//   422 Unprocessable     — a body field is invalid (bad datetime, empty title,
//                           incomplete recurrence pair, unknown override action).
//   500 Internal Error    — an unexpected storage failure (logged, not leaked).
//
// The four small response helpers at the bottom keep those status codes and
// their JSON error bodies consistent across every handler.
//
// How events reach Today: this module only stores events and overrides. The
// surfacing handler (api/surfacing.rs) is what gathers them per day — a one-shot
// event on its start date, a recurring event's materialised instances, minus any
// instance with a Cancel override — and hands them to the same ranking rule as
// tasks. So creating an event here makes it appear on the Today view with no
// extra wiring; the mixed-type list is the whole point of Task 4.
//
// `create_event` is intentionally long (`#[allow(clippy::too_many_lines)]`): it
// threads many optional fields, each with its own validation, and splitting it
// into helpers would scatter the flow rather than clarify it — the same call
// the task API makes for `create_task`.

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
use amity_core::event::{Event, EventBuilder, EventError};
use amity_core::event_override::{EventOverride, OverrideAction};
use amity_core::ids::{EventId, MemberId};
use amity_core::recurrence::RecurrenceRule;

// Storage.
use amity_storage::event::{fetch_event, insert_event, list_events};
use amity_storage::event_instance::{EventInstance, upsert_event_instances};
use amity_storage::event_override::insert_event_override;

// Shared app state (the pool).
use crate::AppState;

// ─── Placeholder member ─────────────────────────────────────────────────────

/// The placeholder member UUID from migration 0001.
///
/// Used as an override's `created_by` until the Member entity is implemented.
///
/// # Panics
///
/// Cannot panic in practice — the string is a validated literal constant.
fn placeholder_member() -> MemberId {
    // The fixed UUID that matches the seeded member row; replaced when real
    // members and authentication land.
    MemberId(
        uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
            .expect("hardcoded UUID is always valid"),
    )
}

// ─── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/events`.
///
/// Only `title` and `start_at` are required. Datetimes are RFC 3339 with an
/// explicit offset (e.g. `"2026-04-27T19:00:00+02:00"`). The recurrence pair
/// must be both present or both absent — half a pair is a 422. `owner`/source
/// are not client-settable: events created here are always native.
#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    /// One-line title; must be non-empty after trimming.
    pub title: String,
    /// Start instant (RFC 3339). For an all-day event this is local midnight in
    /// the event's timezone; surfacing keys the day off its date.
    pub start_at: String,
    /// Optional end instant (RFC 3339); rejected (422) if before `start_at`.
    /// Absent means the event has no meaningful end time.
    pub end_at: Option<String>,
    /// Whether this is an all-day event; absent → a timed event. All-day events
    /// surface without a clock time and lead the day.
    pub all_day: Option<bool>,
    /// IANA timezone name (e.g. "Europe/Amsterdam"); absent → the builder's
    /// Amsterdam default, since the hub shows home time (brief §7).
    pub timezone: Option<String>,
    /// Optional free-form location (e.g. "main hall"); shown, never parsed.
    pub location: Option<String>,
    /// RRULE string (no "RRULE:" prefix), e.g. `"FREQ=WEEKLY;BYDAY=MO"`;
    /// validated against the supported subset and paired with the timezone below.
    pub recurrence_rrule: Option<String>,
    /// IANA timezone for the recurrence; required when an RRULE is set (both or
    /// neither), so DST is handled correctly for the materialised instances.
    pub recurrence_timezone: Option<String>,
}

/// Request body for `POST /api/v1/events/{id}/override`.
///
/// Targets one instance of the event named in the path, on `instance_date`.
#[derive(Debug, Deserialize)]
pub struct CreateOverrideRequest {
    /// The calendar date of the instance to override (YYYY-MM-DD).
    pub instance_date: String,
    /// The action to apply: "cancel" (hide it), "reschedule" (move it), or
    /// "annotate" (note it). An unknown value is a 422.
    pub action: String,
    /// Action data: a new RFC 3339 time for "reschedule", a note for "annotate",
    /// or absent for "cancel".
    pub payload: Option<String>,
}

// ─── Response types ─────────────────────────────────────────────────────────

/// JSON representation of an Event, returned by create/get/list.
///
/// Datetimes are RFC 3339 strings and UUIDs are hyphenated strings; optional
/// fields are omitted from the body entirely when absent (clients check key
/// presence, not `null`). `read_only` is lifted out of the source so the client
/// can decide edit affordances without reading the whole source object.
#[derive(Debug, Serialize)]
pub struct EventResponse {
    /// UUID v7 string (hyphenated) — the event's stable identifier.
    pub id: String,
    /// One-line title, shown verbatim; never blank.
    pub title: String,
    /// Start instant (RFC 3339 with offset).
    pub start_at: String,
    /// End instant (RFC 3339); omitted from the body when the event has no end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_at: Option<String>,
    /// True for an all-day event — the client shows "all day", not a clock time.
    pub all_day: bool,
    /// IANA timezone the event is anchored to; the hub renders home time.
    pub timezone: String,
    /// Free-form location; omitted from the body when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// RRULE string; present only for recurring events (omitted for one-shot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence_rrule: Option<String>,
    /// Recurrence timezone; present iff `recurrence_rrule` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence_timezone: Option<String>,
    /// Source kind: "native" (created here) or "ics" (external feed).
    pub source_kind: String,
    /// Whether the event is read-only (external). The client shows full edit
    /// controls for native events and override-only for read-only ones.
    pub read_only: bool,
    /// Creation timestamp (RFC 3339); immutable after creation.
    pub created_at: String,
    /// Last-modification timestamp (RFC 3339); bumped on every update.
    pub updated_at: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `POST /api/v1/events` — create a native event.
///
/// Flow:
///   1. Parse `start_at`, and validate the recurrence pair against the RRULE
///      subset (both fields present, or both absent).
///   2. Build and validate the domain `Event` (title, timezone, end >= start).
///   3. Persist it; if recurring, materialise instances to a 60-day horizon.
///   4. Return 201 with the created event.
///
/// Events created here are always native (editable); external events arrive via
/// ICS ingestion in a later task.
///
/// Returns HTTP 422 for any validation failure, 500 for unexpected storage errors.
#[allow(clippy::too_many_lines)]
pub async fn create_event(
    State(state): State<AppState>,
    Json(req): Json<CreateEventRequest>,
) -> impl IntoResponse {
    // One `now` for created_at/updated_at and the materialisation horizon, so
    // they all share a single consistent anchor point.
    let now = OffsetDateTime::now_utc();

    // Parse the required start instant.
    let Ok(start_at) = OffsetDateTime::parse(&req.start_at, &Rfc3339) else {
        return unprocessable("start_at: invalid RFC 3339 datetime");
    };

    // Validate the recurrence pair: both present (and valid), or both absent.
    // The result is an Option<RecurrenceRule> reused for materialisation below.
    let recurrence = match (req.recurrence_rrule, req.recurrence_timezone) {
        (Some(rrule), Some(tz)) => {
            // Build and validate against the supported RRULE subset.
            let validated = RecurrenceRule::new(rrule, tz);
            match validated.validate() {
                Ok(()) => Some(validated),
                Err(e) => return unprocessable(&format!("recurrence: {e}")),
            }
        }
        // Neither present: the event is one-shot.
        (None, None) => None,
        // Exactly one of the pair — always a client error.
        _ => {
            return unprocessable(
                "recurrence_rrule and recurrence_timezone must both be provided or both absent",
            );
        }
    };

    // Build the domain event through the validating builder. Title, start, and
    // now are the required fields; the rest are applied conditionally below.
    let mut builder = EventBuilder::new()
        .title(req.title)
        .start_at(start_at)
        .now(now);

    // Apply the optional end instant, rejecting a malformed value.
    if let Some(end_str) = req.end_at {
        match OffsetDateTime::parse(&end_str, &Rfc3339) {
            // Valid: the builder later checks end >= start.
            Ok(dt) => builder = builder.end_at(dt),
            // Unparseable: a field-specific 422.
            Err(_) => return unprocessable("end_at: invalid RFC 3339 datetime"),
        }
    }
    // Apply the optional flags/fields, each only when the client sent it (a
    // missing field leaves the builder default in place).
    // All-day marks a banner event.
    if let Some(all_day) = req.all_day {
        builder = builder.all_day(all_day);
    }
    // Timezone overrides the Amsterdam default.
    if let Some(tz) = req.timezone {
        builder = builder.timezone(tz);
    }
    // Optional location.
    if let Some(loc) = req.location {
        builder = builder.location(loc);
    }
    // Attach the recurrence rule if present (cloned so we keep it for materialising).
    if let Some(ref rule) = recurrence {
        builder = builder.recurrence(rule.clone());
    }

    // Validate all invariants (title, timezone, end >= start) and construct.
    let event = match builder.build() {
        Ok(e) => e,
        // Any builder error is a 422 with the domain message.
        Err(e) => return event_error_response(&e),
    };

    // Persist the event row; a failure here is unexpected.
    if let Err(e) = insert_event(&state.db, &event).await {
        // Log with the storage description; do not leak it to the client.
        tracing::error!(error = %e, "failed to insert event");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Materialise instances for a recurring event; a failure is logged but does
    // not fail the request — the event row is already committed, and a future
    // horizon job would catch up.
    if let Some(ref rule) = recurrence
        && let Err(e) = materialise_event_instances(&state, &event, rule, now).await
    {
        tracing::warn!(event_id = %event.id, error = %e, "initial event materialisation failed");
    }

    // 201 Created with the full event so the client can display it without a GET.
    (StatusCode::CREATED, Json(event_to_response(&event))).into_response()
}

/// `GET /api/v1/events` — list all events, ordered by start time.
///
/// Returns the whole set as a flat array; the Today view does its own per-day
/// filtering in the surfacing handler, so no query parameters are needed here.
///
/// Returns HTTP 200 with a JSON array (possibly empty).
pub async fn list_events_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Delegate to the storage list; map each domain event to its response shape.
    match list_events(&state.db).await {
        // Success: return the array directly, no envelope.
        Ok(events) => {
            // Convert each domain event to its flat JSON representation.
            let responses: Vec<EventResponse> = events.iter().map(event_to_response).collect();
            Json(responses).into_response()
        }
        // Unexpected storage error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list events");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/events/{id}` — fetch one event by UUID.
///
/// The id is validated before the database is touched, so a typo is a cheap 400
/// rather than a round-trip.
///
/// Returns 400 for a malformed id, 404 if not found, 200 with the event otherwise.
pub async fn get_event(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // A non-UUID path parameter is a client error — reject before any DB access.
    let Ok(id) = id_str.parse::<EventId>() else {
        return bad_request("invalid event ID");
    };
    // Distinguish not-found from a storage failure.
    // Look the event up by its validated id.
    match fetch_event(&state.db, id).await {
        // Found: serialise and return 200.
        Ok(Some(event)) => Json(event_to_response(&event)).into_response(),
        // No row for that id: 404.
        Ok(None) => not_found("event not found"),
        // Unexpected database error: log and 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch event");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `POST /api/v1/events/{id}/override` — overlay a change on one instance.
///
/// The event must exist. `action` is cancel|reschedule|annotate; the payload
/// carries the action's data (a new time, a note, or nothing for cancel). The
/// overlay is stored, not applied here — the surfacing layer applies it on read
/// (a `Cancel` hides its instance; reschedule/annotate are a surfacing follow-up).
///
/// Returns 400 for a malformed id, 404 if the event is missing, 422 for a bad
/// action or date, 201 on success.
pub async fn create_override(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<CreateOverrideRequest>,
) -> impl IntoResponse {
    // Validate the target event id from the path before doing anything else.
    let Ok(event_id) = id_str.parse::<EventId>() else {
        return bad_request("invalid event ID");
    };

    // Parse the instance date (YYYY-MM-DD) and the action enum.
    let instance_date = match parse_date(&req.instance_date) {
        Ok(d) => d,
        // Malformed date → 422.
        Err(msg) => return unprocessable(&msg),
    };
    let action = match req.action.parse::<OverrideAction>() {
        Ok(a) => a,
        // Unknown action string → 422.
        Err(e) => return unprocessable(&format!("action: {e}")),
    };

    // The event must exist before we overlay a change on it — a 404 here is
    // clearer than a dangling override with no target.
    match fetch_event(&state.db, event_id).await {
        // Exists: proceed.
        Ok(Some(_)) => {}
        // No such event → 404.
        Ok(None) => return not_found("event not found"),
        // Storage failure → 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch event for override");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Build the overlay (created_by is the placeholder member for now) and
    // persist it; the surfacing layer applies it on read. Overrides are
    // append-only records — a change of mind is a new overlay, not an edit.
    let overlay = EventOverride::new(
        // The event and instance this overlay targets.
        event_id,
        instance_date,
        // The action and its optional payload.
        action,
        req.payload,
        // Who and when (placeholder member for now).
        placeholder_member(),
        OffsetDateTime::now_utc(),
    );
    if let Err(e) = insert_event_override(&state.db, &overlay).await {
        // Storage failure → 500.
        tracing::error!(error = %e, "failed to insert event override");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // 201 with a small confirmation (the new overlay id and applied action) so
    // the client can reflect the change without a re-fetch.
    (
        StatusCode::CREATED,
        Json(json!({ "id": overlay.id.to_string(), "action": action.to_string() })),
    )
        .into_response()
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Materialise a recurring event's instances from `from` to a 60-day horizon.
///
/// The event counterpart to the task-creation materialisation: it reuses the
/// same core materialiser (`amity_core::recurrence_materialiser`) and the
/// event-instance upsert, so a recurring event and a recurring task roll out
/// through one engine. Called once at creation; a future events horizon job
/// would extend the window daily (as the task job does). The holiday-skip set
/// is empty until the NL holiday source lands (ADR-0002 §future-work).
///
/// # Errors
///
/// Returns a message if the core materialiser rejects the rule (it should not —
/// the rule was validated at creation) or the upsert fails. The caller logs it
/// and still returns 201, because the event row is already committed.
async fn materialise_event_instances(
    state: &AppState,
    event: &Event,
    rule: &RecurrenceRule,
    from: OffsetDateTime,
) -> Result<(), String> {
    // The 60-day forward horizon, matching tasks.
    let horizon = from + time::Duration::days(60);
    // No holiday skip set yet — an empty set keeps all dates. The mechanism is
    // in place; the NL holiday source lands later (ADR-0002 §future-work).
    let skip_dates = std::collections::HashSet::new();
    // Core materialiser: RRULE + DTSTART + timezone → scheduled datetimes,
    // preserving wall-clock time across DST like recurring tasks.
    let datetimes = amity_core::recurrence_materialiser::materialise_instances(
        rule,
        from,
        horizon,
        &skip_dates,
    )
    .map_err(|e| format!("materialise_instances: {e}"))?;
    // Wrap each datetime into a storable instance row.
    // Wrap each computed datetime into a storable instance row.
    let instances: Vec<EventInstance> = datetimes
        .into_iter()
        .map(|scheduled_at| EventInstance {
            // Fresh time-ordered id per instance.
            id: uuid::Uuid::now_v7().to_string(),
            // Link back to the parent event.
            event_id: event.id,
            // The computed occurrence time.
            scheduled_at,
        })
        .collect();
    // Persist idempotently — INSERT OR IGNORE dedupes against a prior run.
    upsert_event_instances(&state.db, &instances)
        .await
        .map_err(|e| format!("upsert_event_instances: {e}"))
}

/// Project a domain `Event` into its JSON response shape.
fn event_to_response(event: &Event) -> EventResponse {
    // Format every datetime as RFC 3339; formatting a valid datetime never fails.
    EventResponse {
        // UUID → string.
        id: event.id.to_string(),
        // Title verbatim.
        title: event.title.clone(),
        // Start/end as RFC 3339 strings.
        start_at: event.start_at.format(&Rfc3339).unwrap_or_default(),
        end_at: event.end_at.map(|d| d.format(&Rfc3339).unwrap_or_default()),
        // Flags and passthrough fields.
        all_day: event.all_day,
        timezone: event.timezone.clone(),
        location: event.location.clone(),
        // Split the recurrence rule back into its two wire fields.
        recurrence_rrule: event.recurrence.as_ref().map(|r| r.rrule.clone()),
        recurrence_timezone: event.recurrence.as_ref().map(|r| r.timezone.clone()),
        // Source kind and the lifted read-only flag.
        source_kind: event.source.kind.to_string(),
        read_only: event.source.read_only,
        // Audit timestamps as RFC 3339 strings.
        created_at: event.created_at.format(&Rfc3339).unwrap_or_default(),
        updated_at: event.updated_at.format(&Rfc3339).unwrap_or_default(),
    }
}

/// Map an `EventError` to a 422 response.
///
/// Every builder error is a client-side validation failure, so they all map to
/// Unprocessable Entity with the domain error's message.
fn event_error_response(e: &EventError) -> axum::response::Response {
    // Every builder error is a client-side validation failure → 422.
    // The Display message names the exact problem (empty title, end before
    // start, empty timezone, …).
    unprocessable(&format!("{e}"))
}

/// Parse a `YYYY-MM-DD` string into a `time::Date`.
///
/// Mirrors the date parsing in api/task.rs and api/surfacing.rs so all three
/// accept the same format and produce the same style of error message.
fn parse_date(s: &str) -> Result<time::Date, String> {
    // The bracketed format is a compile-time constant, so building it never fails.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant");
    // Quote the bad value in the error so it is easy to spot in logs.
    time::Date::parse(s, &fmt).map_err(|_| format!("instance_date '{s}': expected YYYY-MM-DD"))
}

/// A 422 Unprocessable Entity response with an error message.
///
/// Used for every client-side validation failure (bad datetime, invalid
/// recurrence, unknown action, empty title).
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
