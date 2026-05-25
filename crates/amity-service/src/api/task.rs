// api/task.rs — HTTP handlers for the Task API.
//
// Endpoints implemented here:
//   POST   /api/v1/tasks                    — create a task
//   GET    /api/v1/tasks                    — list tasks (status/tag/due-before filters)
//   GET    /api/v1/tasks/upcoming           — surfacing query: upcoming instances
//   GET    /api/v1/tasks/{id}               — fetch one task
//   PATCH  /api/v1/tasks/{id}              — update task fields
//   POST   /api/v1/tasks/{id}/complete     — mark done, write CompletionLog
//   POST   /api/v1/tasks/{id}/skip         — mark skipped, write CompletionLog
//   POST   /api/v1/tasks/{id}/assignee     — change current_assignee_id
//   GET    /api/v1/tasks/{id}/history      — list CompletionLog entries for task
//
// All handlers follow the same pattern as api/inbox.rs:
//   parse request → validate domain rules → storage operations → serialise response.
//
// Error handling convention (used throughout this file):
//   HTTP 400 Bad Request       — path parameter is not a valid UUID.
//   HTTP 404 Not Found         — UUID is valid but no matching row in the database.
//   HTTP 422 Unprocessable     — request body violates a domain invariant
//                                (empty title, invalid status, out-of-range effort, etc.).
//   HTTP 500 Internal Error    — unexpected storage failure; details are logged but
//                                not returned to the client to avoid leaking internals.
//
// Placeholder member:
//   The Member entity is not yet implemented. All handlers that need a member ID
//   use the placeholder UUID from migration 0001 (`placeholder_member()`). Every
//   `owner_id`, `captured_by`, and `completed_by` field is this placeholder until
//   the Member entity lands in a later task.
//
// Recurrence:
//   Tasks with a recurrence rule have their instances materialised at creation time
//   (up to a 60-day horizon) via `materialise_and_store_instances`. A background job
//   (not yet implemented) will extend the horizon on a daily schedule. If initial
//   materialisation fails, the task itself is still persisted — the background job
//   will catch up on its next run.
//
// Note on handler length:
//   The `create_task` and `patch_task` handlers are intentionally long. They handle
//   many optional fields with independent validation, and factoring them out would
//   reduce readability by spreading the logic across many small helpers. The
//   `#[allow(clippy::too_many_lines)]` attribute is used for these two handlers.

// axum's JSON extractor/response wrapper — used by every handler in this module.
use axum::Json;
// `Path` extracts `{id}` from the URL; `Query` extracts `?key=value` params; `State`
// injects the shared `AppState` (containing the database pool).
use axum::extract::{Path, Query, State};
// HTTP status codes used in response tuples (e.g. `(StatusCode::NOT_FOUND, Json(...))`).
use axum::http::StatusCode;
// Sealed trait implemented by tuples, `Json<T>`, `StatusCode`, etc. for response dispatch.
use axum::response::IntoResponse;
// `Deserialize` for request body structs; `Serialize` for response body structs.
use serde::{Deserialize, Serialize};
// `OffsetDateTime` is the project's canonical timestamp type; all datetimes carry a UTC offset.
use time::OffsetDateTime;
// RFC 3339 format descriptor used to convert `OffsetDateTime` to/from TEXT strings.
use time::format_description::well_known::Rfc3339;

// `CompletionLog` is the immutable record written on task completion or skip events.
use amity_core::completion_log::CompletionLog;
// Typed UUID newtypes: `MemberId` for member references, `TaskId` for task primary keys.
use amity_core::ids::{MemberId, TaskId};
// `RecurrenceRule` holds the RRULE string and IANA timezone for recurring tasks.
use amity_core::recurrence::RecurrenceRule;
// `Task` is the domain struct; `TaskBuilder` enforces invariants; `TaskStatus` is the lifecycle enum.
use amity_core::task::{Task, TaskBuilder, TaskStatus};

// `list_completion_logs_for_task` is used by the `/history` endpoint.
use amity_storage::completion_log::list_completion_logs_for_task;
// Task repository functions: CRUD plus the `mark_*` operations that write CompletionLog rows.
use amity_storage::task::{
    TaskFilter, fetch_task, insert_task, list_tasks, mark_task_done, mark_task_skipped, update_task,
};
// Task instance functions: materialise, list upcoming, prune, and delete-on-change.
use amity_storage::task_instance::{
    TaskInstance, delete_future_instances, list_upcoming_instances, upsert_task_instances,
};

// `AppState` carries the shared `SqlitePool` injected by `build_app`.
use crate::AppState;

// ─── Placeholder member ───────────────────────────────────────────────────────

/// The placeholder member UUID from migration 0001.
///
/// Used as `owner_id` and `completed_by` until the Member entity is implemented.
/// All handler functions that need a member ID call this function. The value is
/// a compile-time constant so parsing never fails.
///
/// # Panics
///
/// Panics if the hardcoded UUID string is not valid — cannot happen in practice
/// because the string is a literal constant that was validated when it was written.
fn placeholder_member() -> MemberId {
    // The fixed UUID that matches the seeded row from migration 0001.
    // This will be replaced once the Member entity and authentication land.
    MemberId(
        uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
            .expect("hardcoded UUID is always valid"),
    )
}

// ─── Request types ────────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/tasks`.
///
/// All fields except `title` are optional. `owner_id` is not exposed as a
/// request field — it always defaults to the placeholder member for now.
///
/// The `recurrence_rrule` and `recurrence_timezone` fields must be either both
/// present or both absent. Providing only one returns HTTP 422.
///
/// All datetime strings must be RFC 3339 with an explicit UTC offset
/// (e.g. `"2026-06-01T08:00:00+02:00"`); bare UTC (`"Z"` suffix) is also valid.
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    /// One-line summary; must not be blank after trimming.
    ///
    /// The server stores the title verbatim (no normalisation beyond the
    /// non-empty check). Clients should trim whitespace before sending.
    pub title: String,
    /// Optional free-form notes; absent → no notes attached to the task.
    pub notes: Option<String>,
    /// Latest meaningful completion time (RFC 3339 with offset).
    ///
    /// This is a soft signal — the system surfaces it without pressure (brief §3).
    pub due_by: Option<String>,
    /// Earliest meaningful start time (RFC 3339 with offset); absent → available now.
    pub earliest_at: Option<String>,
    /// Effort estimate 1-5; absent → not estimated.
    ///
    /// 1 = a few minutes; 5 = most of an afternoon. The system never derives
    /// a workload score from effort — it is a hint for the household.
    pub effort: Option<u8>,
    /// Importance rank 1-5; absent → not ranked.
    pub priority: Option<u8>,
    /// Free-form tags; normalised (trim + lowercase) by the domain layer.
    ///
    /// Tags drive default behaviours: `chore`/`household` trigger holiday-skip;
    /// `outdoor` adds a weather context hint (brief §8.3).
    pub tags: Option<Vec<String>>,
    /// RFC 5545 RRULE string (no "RRULE:" prefix); must be paired with timezone.
    ///
    /// Example: `"FREQ=WEEKLY;BYDAY=TH"`. The supported subset is documented
    /// in `amity_core::recurrence::validate_rrule_string`.
    pub recurrence_rrule: Option<String>,
    /// IANA timezone name (e.g. "Europe/Amsterdam"); required when rrule is set.
    ///
    /// Controls DST-correct instance materialisation. Defaults to Amsterdam per brief §7.
    pub recurrence_timezone: Option<String>,
}

/// Request body for `PATCH /api/v1/tasks/{id}`.
///
/// All fields are optional. Only fields present in the JSON body are applied.
/// Sending `null` for a clearable field (e.g. `due_by`) resets it to absent.
///
/// The semantics differ by field type:
/// - `title`: absent = no change; blank = rejected (422).
/// - `due_by` / `earliest_at`: absent = no change; empty string `""` = clear the field.
/// - `tags`: absent = no change; empty array `[]` = clear all tags.
/// - `recurrence_rrule` + `recurrence_timezone`: absent pair = no change; new pair = replace
///   and re-materialise instances; one without the other = 422.
#[derive(Debug, Deserialize)]
pub struct UpdateTaskRequest {
    /// Replacement title; rejected when blank, ignored when absent.
    pub title: Option<String>,
    /// Replacement notes; absent = leave unchanged; empty string = clear.
    pub notes: Option<String>,
    /// Replacement status string (open|doing|done|skipped).
    ///
    /// The handler parses this via `TaskStatus::from_str`; unknown values return 422.
    pub status: Option<String>,
    /// Replacement `due_by` (RFC 3339); absent or empty string = clear.
    pub due_by: Option<String>,
    /// Replacement `earliest_at` (RFC 3339); absent or empty string = clear.
    pub earliest_at: Option<String>,
    /// Replacement effort 1-5; absent = leave unchanged.
    pub effort: Option<u8>,
    /// Replacement priority 1-5; absent = leave unchanged.
    pub priority: Option<u8>,
    /// Replacement tag list (replaces the whole list, not a merge).
    ///
    /// Send `[]` to remove all tags from the task.
    pub tags: Option<Vec<String>>,
    /// Replacement RRULE string; must be paired with `recurrence_timezone`.
    pub recurrence_rrule: Option<String>,
    /// Replacement IANA timezone; must be paired with `recurrence_rrule`.
    pub recurrence_timezone: Option<String>,
}

/// Request body for `POST /api/v1/tasks/{id}/complete` and `.../skip`.
///
/// Both the complete and skip endpoints share this request shape because
/// the information needed is identical — only the `skipped` flag written
/// to the `CompletionLog` differs between the two endpoints.
#[derive(Debug, Deserialize)]
pub struct CompleteTaskRequest {
    /// Scheduled date of the instance being resolved (YYYY-MM-DD).
    ///
    /// For recurring tasks: the date from `task_instances.scheduled_at`.
    /// For one-shot tasks: the task's `due_by` date if set, or today otherwise.
    /// The server parses this as `time::Date` and rejects non-YYYY-MM-DD strings with 422.
    pub instance_date: String,
    /// Optional free-form note from the member (e.g. "did it together with Anna").
    ///
    /// The system never parses or acts on this field — it is pure audit context.
    /// Notes are stored in the `CompletionLog` and displayed in the history view.
    pub notes: Option<String>,
}

/// Request body for `POST /api/v1/tasks/{id}/assignee`.
///
/// This endpoint is the "one-tap reassignment" affordance (brief §8.2).
/// It is separate from PATCH to keep the common case fast — changing who does
/// a task should require no more than a single confirmation tap on the hub.
#[derive(Debug, Deserialize)]
pub struct ChangeAssigneeRequest {
    /// UUID of the member to set as `current_assignee_id`; `null` clears it.
    ///
    /// Send `"member_id": null` to remove the current assignee without setting a new one.
    /// Send a UUID string to set a specific member as the default assignee.
    pub member_id: Option<String>,
}

/// Query parameters for `GET /api/v1/tasks`.
///
/// All parameters are optional and AND-ed together. An empty query string
/// returns all tasks ordered by `created_at DESC`.
#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    /// Restrict to tasks in this lifecycle state (open|doing|done|skipped).
    ///
    /// The value must match one of the four known status strings; unknown values return 400.
    pub status: Option<String>,
    /// Restrict to tasks carrying this tag (exact, normalised match).
    ///
    /// The match is case-insensitive (tags are normalised to lowercase on write).
    /// Only tasks containing this exact tag string in their tag list are returned.
    pub tag: Option<String>,
    /// Restrict to tasks whose `due_by` is on or before this RFC 3339 datetime.
    ///
    /// Tasks with no `due_by` are never returned by this filter.
    pub due_before: Option<String>,
}

/// Query parameters for `GET /api/v1/tasks/upcoming`.
///
/// The `limit` parameter controls how many upcoming instances are returned
/// in the sorted, cross-task list. The default is tuned for the hub's touch screen.
#[derive(Debug, Deserialize)]
pub struct UpcomingQuery {
    /// How many upcoming instances to return. Defaults to 20; capped at 100.
    ///
    /// A value of 20 comfortably fills a one-day or one-week view on the hub.
    /// The server enforces the cap even if the client requests more than 100.
    #[serde(default = "default_upcoming_limit")]
    pub limit: i64,
}

// Called by serde when `limit` is absent from the query string.
// 20 fits the hub touch screen without paging; clients can request up to 100.
// The value is an i64 to match the type used by `list_upcoming_instances`.
fn default_upcoming_limit() -> i64 {
    // 20 instances gives one full day's view at a "daily" recurrence frequency.
    20
}

// ─── Response types ───────────────────────────────────────────────────────────

/// JSON representation of a Task returned by all task endpoints.
///
/// Optional fields are omitted from the JSON body entirely when absent.
/// Clients should not check for `"key": null` — they check for key presence.
///
/// All UUID fields are hyphenated lowercase strings (e.g. `"018f1a2b-0000-7000-8000-000000000010"`).
/// All datetime fields are RFC 3339 strings with an explicit UTC offset
/// (e.g. `"2026-06-01T08:00:00+02:00"`).
///
/// The response shape is identical for the create, get, patch, and change-assignee endpoints;
/// clients can use the same deserialisers for all of them.
#[derive(Debug, Serialize)]
pub struct TaskResponse {
    /// UUID v7 string (hyphenated) — globally unique, time-ordered identifier.
    pub id: String,
    /// One-line summary — always present; never blank.
    pub title: String,
    /// Free-form notes; omitted when no notes are attached.
    ///
    /// Clients should render this as plain prose or a bullet list;
    /// the system never parses or acts on the content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Lifecycle state: open|doing|done|skipped.
    ///
    /// For recurring tasks, `status` reflects the whole-series lifecycle, not
    /// the per-instance state. Per-instance completion history lives in the
    /// `/history` endpoint (backed by `CompletionLog`).
    pub status: String,
    /// Latest meaningful completion time (RFC 3339); omitted when open-ended.
    ///
    /// This is a soft signal — the system surfaces it without generating pressure
    /// or notifications when it is missed. See brief §3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_by: Option<String>,
    /// Earliest start time (RFC 3339); omitted when available immediately.
    ///
    /// Tasks with `earliest_at` in the future are hidden from the Today view
    /// until that moment arrives, keeping the dashboard uncluttered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_at: Option<String>,
    /// Effort estimate 1-5; omitted when not estimated.
    ///
    /// 1 = a few minutes; 5 = most of an afternoon. A soft signal for surfacing
    /// order and personal focus — never used to compute workload scores.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<u8>,
    /// Importance rank 1-5; omitted when not ranked.
    ///
    /// Higher-priority tasks are surfaced more prominently in the Today view.
    /// The system does not use this to generate pressure or escalating notifications.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Normalised tags (trimmed, lowercased); empty array when none are set.
    ///
    /// Tags drive default behaviours: `chore`/`household` trigger holiday-skip;
    /// `outdoor` adds a weather context hint; `kid` controls Kid Mode visibility.
    pub tags: Vec<String>,
    /// UUID string of the member who owns this task (responsible for it existing).
    pub owner_id: String,
    /// UUID strings of members actively assigned (doing the work).
    pub assignee_ids: Vec<String>,
    /// UUID strings of members eligible to take this task; empty means anyone.
    pub eligible_member_ids: Vec<String>,
    /// UUID string of the default displayed assignee; omitted when unset.
    ///
    /// Freely changeable with a single tap via `POST /assignee`. Not an obligation;
    /// a default that the household can override without ceremony (brief §8.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_assignee_id: Option<String>,
    /// RRULE string (no "RRULE:" prefix); omitted for one-shot tasks.
    ///
    /// Present only when the task has a recurrence rule. The client can display
    /// this string to show the recurrence pattern (e.g. "FREQ=WEEKLY;BYDAY=TH").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence_rrule: Option<String>,
    /// IANA timezone for DST-correct materialisation; omitted for one-shot tasks.
    ///
    /// Always present when `recurrence_rrule` is present; absent when it is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence_timezone: Option<String>,
    /// Creation timestamp (RFC 3339) — immutable after the task is created.
    pub created_at: String,
    /// Last-modification timestamp (RFC 3339) — bumped on every update or completion.
    pub updated_at: String,
}

/// JSON representation of a `CompletionLog` entry.
///
/// Returned by the `/complete`, `/skip`, and `/history` endpoints.
///
/// `CompletionLog` entries are immutable — the system never updates or deletes
/// them. The history is an append-only audit trail of what the household did
/// and when. Corrections are recorded by adding a new entry with a note,
/// not by editing the incorrect one.
///
/// The `skipped` field distinguishes a skip event from a genuine completion.
/// Both event types appear in the `/history` endpoint so the household can see
/// the full picture — "skipping" is a first-class fact (brief §8.2).
#[derive(Debug, Serialize)]
pub struct CompletionLogResponse {
    /// UUID v7 string — unique ID for this immutable log entry.
    ///
    /// The UUID v7 format embeds a timestamp, so log entries are naturally
    /// time-ordered by their ID even without reading `completed_at`.
    pub id: String,
    /// UUID string of the task that was acted on.
    pub task_id: String,
    /// Calendar date of the instance (YYYY-MM-DD).
    ///
    /// For recurring tasks: the scheduled date from `task_instances.scheduled_at`.
    /// For one-shot tasks: the task's `due_by` date if set, or today at completion time.
    pub instance_date: String,
    /// UUID string of the member who performed the action.
    ///
    /// This is the actor — the member who pressed the "done" or "skip" button.
    /// Currently always the placeholder member; real member identity comes later.
    pub completed_by: String,
    /// Precise timestamp of the action (RFC 3339 with offset).
    ///
    /// Includes the actor's local UTC offset so the household can see whether
    /// the task was done at a sensible wall-clock time.
    pub completed_at: String,
    /// `true` if the instance was skipped; `false` if completed.
    ///
    /// The client uses this to render skip events differently from completions
    /// in the history view (e.g. grey "skipped" vs green "done" chip).
    pub skipped: bool,
    /// Optional free-form context; omitted when absent.
    ///
    /// E.g. "did it together with Anna" or "chimney sweep rescheduled to Nov".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// JSON representation of an upcoming recurring task instance.
///
/// Returned by `GET /api/v1/tasks/upcoming`. Title and status are
/// denormalised from the parent task to allow rendering without a
/// second fetch per instance.
///
/// The `instance_id` identifies the row in `task_instances`. The client can
/// use `task_id` to navigate to the parent task's detail view.
///
/// The list is sorted by `scheduled_at` ascending — nearest instances first.
/// One-shot tasks are not included (they have no `task_instances` rows).
#[derive(Debug, Serialize)]
pub struct UpcomingInstanceResponse {
    /// UUID string of the `task_instances` row — uniquely identifies this occurrence.
    pub instance_id: String,
    /// UUID string of the parent recurring task.
    ///
    /// Use this to navigate to the task detail view or to call `/complete` or `/skip`.
    pub task_id: String,
    /// Scheduled datetime for this occurrence (RFC 3339).
    ///
    /// The datetime includes the local UTC offset from the IANA timezone attached
    /// to the parent task's recurrence rule. 08:00 Amsterdam time is "+02:00" in
    /// summer and "+01:00" in winter, even for the same weekly rule.
    pub scheduled_at: String,
    /// Default assignee for this instance; omitted when unset.
    ///
    /// Inherited from the parent task's `current_assignee_id` at materialisation time.
    /// Can be overridden per-instance via the `/assignee` endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_assignee_id: Option<String>,
    /// Task title — denormalised from the parent task to avoid a per-instance fetch.
    pub title: String,
    /// Parent task lifecycle status — typically "open" for upcoming tasks.
    ///
    /// A "done" or "skipped" status indicates the parent task series has ended;
    /// such tasks are typically excluded from the upcoming view by the `Open` filter.
    pub status: String,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /api/v1/tasks` — create a new task.
///
/// Flow:
///   1. Validate the recurrence rule pair (both must be present, or both absent).
///   2. Build and validate the domain `Task` (title, effort, priority ranges).
///   3. Persist the task via `insert_task`.
///   4. If recurring, materialise instances up to a 60-day horizon.
///   5. Return HTTP 201 with the created task.
///
/// Returns HTTP 422 for any domain validation failure.
/// Returns HTTP 500 for unexpected storage errors.
#[allow(clippy::too_many_lines)]
pub async fn create_task(
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    // Capture the current time once so `created_at`, `updated_at`, and the
    // materialisation horizon all share the same anchor point.
    let now = OffsetDateTime::now_utc();

    // ── Validate the recurrence rule pair ─────────────────────────────────────
    //
    // The recurrence rule is stored as two separate columns in the database
    // (`recurrence_rrule`, `recurrence_timezone`) but must arrive as a pair.
    // Providing only one of them is a client error.
    let recurrence = match (req.recurrence_rrule, req.recurrence_timezone) {
        // Both present: construct the rule struct and validate it against the
        // supported RRULE subset (DAILY/WEEKLY/MONTHLY/YEARLY with standard modifiers).
        (Some(rrule), Some(tz)) => {
            let validated = RecurrenceRule::new(rrule, tz);
            match validated.validate() {
                // Rule is within the supported subset: accept it.
                Ok(()) => Some(validated),
                // Rule uses an unsupported feature (BYWEEKNO, sub-daily FREQ, etc.):
                // reject with 422 so the client knows to fix the rule, not retry.
                Err(e) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(serde_json::json!({ "error": format!("recurrence: {e}") })),
                    )
                        .into_response();
                }
            }
        }
        // Neither present: the task is one-shot (no recurrence).
        (None, None) => None,
        // Exactly one present: the pair is incomplete — always a client error.
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "recurrence_rrule and recurrence_timezone must both be provided or both absent"
                })),
            )
                .into_response();
        }
    };

    // ── Build the domain Task ─────────────────────────────────────────────────
    //
    // The `TaskBuilder` is the single source of domain validation.
    // It enforces: non-empty title, valid effort/priority range (1-5),
    // and that required fields (owner_id, now) are set before `.build()`.
    let mut builder = TaskBuilder::new()
        .title(req.title)
        // Use the placeholder member for `owner_id` until Member entity lands.
        .owner_id(placeholder_member())
        // Set both `created_at` and `updated_at` from the captured timestamp.
        .now(now);

    // Apply optional notes if provided.
    // Notes are stored verbatim — no truncation or normalisation.
    if let Some(notes) = req.notes {
        builder = builder.notes(notes);
    }
    // Attach the recurrence rule if the task is recurring.
    // Cloning the validated rule into the builder here avoids a move conflict
    // when we also need `recurrence` later for the materialisation step.
    if let Some(ref rule) = recurrence {
        builder = builder.recurrence(rule.clone());
    }
    // Add each tag through the builder so normalisation (trim + lowercase) is applied.
    // `unwrap_or_default()` treats an absent `tags` field as an empty list.
    for tag in req.tags.unwrap_or_default() {
        builder = builder.tag(tag);
    }

    // Parse and apply the optional due-by datetime.
    // The parse is done here rather than via a generic helper so the field name
    // "due_by" can appear in the specific 422 error message.
    if let Some(due_str) = req.due_by {
        match OffsetDateTime::parse(&due_str, &Rfc3339) {
            // RFC 3339 parse succeeded: attach to the builder.
            Ok(dt) => builder = builder.due_by(dt),
            // The string is not valid RFC 3339 — return a field-specific 422.
            Err(_) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": "due_by: invalid RFC 3339 datetime" })),
                )
                    .into_response();
            }
        }
    }

    // Parse and apply the optional earliest-at datetime (same pattern as due_by).
    // Absent `earliest_at` means the task is immediately available in the Today view.
    if let Some(ea_str) = req.earliest_at {
        match OffsetDateTime::parse(&ea_str, &Rfc3339) {
            // Valid RFC 3339: the task will not appear in the Today view before this moment.
            Ok(dt) => builder = builder.earliest_at(dt),
            // Invalid RFC 3339: return a field-specific 422.
            Err(_) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": "earliest_at: invalid RFC 3339 datetime" })),
                )
                    .into_response();
            }
        }
    }

    // Apply effort — the builder validates that the value is in the 1-5 range.
    // `builder.effort()` returns the builder on success and a `TaskError` on failure.
    // We move the builder into the call and get it back on success, or bail on failure.
    if let Some(effort) = req.effort {
        match builder.effort(effort) {
            Ok(b) => builder = b,
            // Out-of-range: the domain error message includes the allowed range.
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("effort: {e}") })),
                )
                    .into_response();
            }
        }
    }

    // Apply priority — same 1-5 range validation as effort.
    // Both newtypes share the same valid range; the error message differs.
    if let Some(priority) = req.priority {
        match builder.priority(priority) {
            Ok(b) => builder = b,
            // Out-of-range value: 422 with the domain error message.
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("priority: {e}") })),
                )
                    .into_response();
            }
        }
    }

    // Build the final domain type; this call validates that title is non-empty
    // and that all required builder fields were set.
    let task = match builder.build() {
        // All invariants satisfied: proceed with the constructed Task.
        Ok(t) => t,
        // A domain invariant was violated — return the error message from the domain layer.
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": format!("{e}") })),
            )
                .into_response();
        }
    };

    // ── Persist the task ──────────────────────────────────────────────────────
    //
    // All validation passed; write the task row to the database.
    // A failure here is unexpected (constraint violation, disk error, etc.).
    if let Err(e) = insert_task(&state.db, &task).await {
        // Log the error with the storage layer's description for debugging.
        tracing::error!(error = %e, "failed to insert task");
        // Return 500 — do not leak the storage error to the client.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // ── Materialise recurring instances ───────────────────────────────────────
    //
    // For recurring tasks, pre-materialise the first 60 days of scheduled instances
    // so the `/upcoming` endpoint has data to return immediately.
    // If this fails (e.g. invalid timezone not caught by validation), we log a
    // warning but still return 201 — the task was persisted. The background job
    // will retry materialisation on its next scheduled run.
    if let Some(ref rule) = recurrence
        && let Err(e) = materialise_and_store_instances(&state, &task, rule, now).await
    {
        tracing::warn!(task_id = %task.id, error = %e, "initial recurrence materialisation failed");
    }

    // Return 201 Created with the full task body so the client can display it
    // without a separate GET request.
    (StatusCode::CREATED, Json(task_to_response(&task))).into_response()
}

/// `GET /api/v1/tasks` — list tasks with optional filters.
///
/// All query parameters are optional and AND-ed together. An empty set of
/// filters returns all tasks. Results are ordered `created_at DESC`.
///
/// Returns HTTP 400 if a filter parameter is malformed.
/// Returns HTTP 200 with a JSON array (possibly empty) on success.
pub async fn list_tasks_handler(
    State(state): State<AppState>,
    Query(params): Query<ListTasksQuery>,
) -> impl IntoResponse {
    // Parse the status filter string (e.g. "open") into the domain enum.
    // An unknown status string is a client error (400), not a domain error (422),
    // because it is a query parameter rather than a body field.
    let status = match params.status.as_deref() {
        Some(s) => match s.parse::<TaskStatus>() {
            // Known status: use it as the filter value.
            Ok(ts) => Some(ts),
            // Unknown status string: 400 with the unrecognised value echoed back.
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown status: {s}") })),
                )
                    .into_response();
            }
        },
        // Absent: no status filter — match any lifecycle state.
        None => None,
    };

    // Parse the optional due_before datetime filter.
    // Must be RFC 3339 with a timezone offset (e.g. "2026-06-01T00:00:00+02:00").
    let due_before = match params.due_before.as_deref() {
        Some(s) => match OffsetDateTime::parse(s, &Rfc3339) {
            // Valid RFC 3339: use as the upper bound for `due_by`.
            Ok(dt) => Some(dt),
            // Malformed datetime string: 400 with field reference.
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "due_before: invalid RFC 3339 datetime" })),
                )
                    .into_response();
            }
        },
        // Absent: no due_before filter — return tasks with any `due_by` value.
        None => None,
    };

    // Assemble the filter struct. Fields not set match all rows.
    let filter = TaskFilter {
        status,
        // Pass the tag string through directly; the storage layer handles the
        // JSON array substring search.
        tag: params.tag,
        due_before,
    };

    // Execute the filtered list query and map results to the response type.
    match list_tasks(&state.db, &filter).await {
        // Success: convert each domain Task to its flat JSON representation.
        Ok(tasks) => {
            let responses: Vec<TaskResponse> = tasks.iter().map(task_to_response).collect();
            // HTTP 200 with the array directly — no envelope wrapper.
            Json(responses).into_response()
        }
        // Unexpected storage error: log it and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list tasks");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/v1/tasks/upcoming` — return upcoming recurring task instances.
///
/// Returns up to `limit` instances with `scheduled_at >= now`, across all open
/// recurring tasks, sorted by `scheduled_at` ascending. Title and status are
/// included per-instance to avoid a second round-trip in the client.
///
/// Returns HTTP 200 with a JSON array (possibly empty if no recurring tasks exist).
pub async fn list_upcoming(
    State(state): State<AppState>,
    Query(params): Query<UpcomingQuery>,
) -> impl IntoResponse {
    // Cap the limit at 100 to prevent accidentally large responses.
    // The default is 20; clients should specify an explicit limit for larger windows.
    let limit = params.limit.min(100);
    // Everything at or after this moment is "upcoming".
    let now = OffsetDateTime::now_utc();

    // Fetch all open tasks as the candidate set.
    // One-shot tasks are included here but filtered out in the loop below
    // because they have no rows in `task_instances`.
    let filter = TaskFilter {
        status: Some(TaskStatus::Open),
        tag: None,
        due_before: None,
    };
    let tasks = match list_tasks(&state.db, &filter).await {
        // Got the candidate task list: continue to fetch instances.
        Ok(t) => t,
        // Storage failure when fetching tasks: cannot proceed, return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list tasks for upcoming query");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Gather upcoming instances from all recurring tasks.
    let mut upcoming: Vec<UpcomingInstanceResponse> = Vec::new();
    for task in &tasks {
        // One-shot tasks have no `task_instances` rows; skip them.
        if task.recurrence.is_none() {
            continue;
        }
        // Fetch the nearest `limit` future instances for this recurring task.
        let instances = match list_upcoming_instances(&state.db, task.id, now, limit).await {
            Ok(i) => i,
            // Per-task failure: log a warning and continue with the remaining tasks.
            // The endpoint still returns data for the tasks that did succeed.
            Err(e) => {
                tracing::warn!(task_id = %task.id, error = %e, "failed to fetch upcoming instances");
                continue;
            }
        };
        // Convert each instance row to its response shape, denormalising the parent
        // task's title and status to avoid a per-instance database round-trip.
        for inst in instances {
            // Format the scheduled time as RFC 3339 for the JSON response.
            let scheduled_str = inst.scheduled_at.format(&Rfc3339).unwrap_or_default();
            upcoming.push(UpcomingInstanceResponse {
                // The instance's own unique ID (from `task_instances.id`).
                instance_id: inst.id,
                // The parent task's ID (for client navigation to the task detail view).
                task_id: task.id.to_string(),
                scheduled_at: scheduled_str,
                // Current default assignee for this instance, if one is set.
                current_assignee_id: inst.current_assignee_id.map(|m| m.to_string()),
                // Denormalised from the parent task — avoids a second fetch per instance.
                title: task.title.clone(),
                status: task.status.to_string(),
            });
        }
    }

    // Sort the merged result by `scheduled_at` ascending so the nearest instances
    // appear first regardless of which task they belong to.
    // String comparison is correct here because RFC 3339 with a consistent offset
    // is lexicographically ordered — earlier timestamps sort before later ones.
    upcoming.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at));
    // Trim to the requested limit after the cross-task sort.
    // `limit` is already capped at 100, which fits safely in usize on all targets.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    upcoming.truncate(limit as usize);

    // Return the sorted, trimmed list as HTTP 200.
    Json(upcoming).into_response()
}

/// `GET /api/v1/tasks/{id}` — fetch one task by UUID.
///
/// Returns HTTP 400 for a malformed UUID path parameter.
/// Returns HTTP 404 if the UUID is valid but no task with that ID exists.
/// Returns HTTP 200 with the task body on success.
pub async fn get_task(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // Validate the path parameter. A non-UUID value is always a client error (400).
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // Fetch the task, distinguishing between "not found" and storage failure.
    match fetch_task(&state.db, id).await {
        // Task found: serialise and return 200.
        Ok(Some(task)) => Json(task_to_response(&task)).into_response(),
        // No row for that ID: 404 with a terse message.
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "task not found" })),
        )
            .into_response(),
        // Unexpected database error: log and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch task");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `PATCH /api/v1/tasks/{id}` — update mutable task fields.
///
/// Only fields present in the JSON body are modified; absent fields are left
/// unchanged. If the recurrence rule changes, all future instances are deleted
/// and re-materialised from the new rule (ADR-0002 §mid-flight-recurrence-change).
///
/// Returns HTTP 200 with the updated task body on success.
/// Returns HTTP 404 if the task does not exist.
/// Returns HTTP 422 for any domain validation failure.
#[allow(clippy::too_many_lines)]
pub async fn patch_task(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<UpdateTaskRequest>,
) -> impl IntoResponse {
    // Validate the UUID path parameter before touching the database.
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // ── Load the existing task ────────────────────────────────────────────────
    //
    // We load the full current task and apply the partial update on top of it.
    // This means clients send only the fields they want to change, and the
    // storage UPDATE still writes all columns (to avoid partial-update complexity).
    let mut task = match fetch_task(&state.db, id).await {
        Ok(Some(t)) => t,
        // No task for that ID: 404 before attempting any changes.
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
                .into_response();
        }
        // Storage error while loading — abort.
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch task for update");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // ── Apply field updates ───────────────────────────────────────────────────
    //
    // Each field update is guarded by `if let Some(...)` so only provided fields
    // are changed. Absent fields leave the loaded task value unchanged.

    // Title: reject an empty replacement but allow any non-empty string.
    if let Some(title) = req.title {
        if title.trim().is_empty() {
            // An empty title is always a domain error regardless of prior value.
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "title must not be empty" })),
            )
                .into_response();
        }
        // Non-empty: update the title in the loaded task struct.
        task.title = title;
    }

    // Notes: absent = leave unchanged; non-empty string = update; empty = clear.
    // An empty string explicitly removes the notes rather than setting them to "".
    if let Some(notes) = req.notes {
        task.notes = if notes.is_empty() { None } else { Some(notes) };
    }

    // Status: parse the lifecycle state string and update if valid.
    // The service layer does not enforce state-machine transitions — the household
    // can move a task to any state (e.g. reopen a done task).
    if let Some(status_str) = req.status {
        match status_str.parse::<TaskStatus>() {
            // Recognised status: apply the transition.
            Ok(s) => task.status = s,
            // Unrecognised: 422 with the invalid value echoed for debugging.
            Err(_) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("unknown status: {status_str}") })),
                )
                    .into_response();
            }
        }
    }

    // due_by: empty string clears the field; non-empty must parse as RFC 3339.
    // The empty-string convention avoids the complexity of a JSON `null` vs absent field.
    if let Some(due_str) = req.due_by {
        if due_str.is_empty() {
            // Explicit clear: the task becomes open-ended (no deadline).
            task.due_by = None;
        } else {
            match OffsetDateTime::parse(&due_str, &Rfc3339) {
                // Valid RFC 3339: set the new deadline.
                Ok(dt) => task.due_by = Some(dt),
                // Invalid datetime format: 422 with the field name.
                Err(_) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(serde_json::json!({ "error": "due_by: invalid RFC 3339 datetime" })),
                    )
                        .into_response();
                }
            }
        }
    }

    // earliest_at: same pattern as due_by above.
    // Empty string removes the "not yet available" window; RFC 3339 sets a new one.
    if let Some(ea_str) = req.earliest_at {
        if ea_str.is_empty() {
            // Explicit clear: task becomes immediately available in the Today view.
            task.earliest_at = None;
        } else {
            match OffsetDateTime::parse(&ea_str, &Rfc3339) {
                Ok(dt) => task.earliest_at = Some(dt),
                Err(_) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(serde_json::json!({ "error": "earliest_at: invalid RFC 3339 datetime" })),
                    )
                        .into_response();
                }
            }
        }
    }

    // Effort: validate the 1-5 range and update.
    // Unlike the builder pattern, we call `EffortLevel::new` directly here because
    // we are modifying an existing task, not constructing a new one via the builder.
    if let Some(effort_val) = req.effort {
        match amity_core::task::EffortLevel::new(effort_val) {
            Ok(e) => task.effort = Some(e),
            // Out-of-range: 422 with the domain error (includes the bad value).
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("effort: {err}") })),
                )
                    .into_response();
            }
        }
    }

    // Priority: same range validation as effort.
    // Sending a priority value of 0 or >5 is always a 422; in-range values replace
    // the existing priority or set one where none existed before.
    if let Some(priority_val) = req.priority {
        match amity_core::task::Priority::new(priority_val) {
            Ok(p) => task.priority = Some(p),
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("priority: {err}") })),
                )
                    .into_response();
            }
        }
    }

    // Tags: replace the whole list (not a merge). Normalise with trim + lowercase
    // to be consistent with the TagBuilder normalisation on creation. An empty Vec
    // removes all tags from the task.
    if let Some(tags) = req.tags {
        // Each tag is trimmed and lowercased; blank strings after trimming are dropped.
        task.tags = tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
    }

    // ── Recurrence update ─────────────────────────────────────────────────────
    //
    // Changing the recurrence rule is more complex than other field updates
    // because it invalidates previously-materialised future instances. We track
    // whether the rule actually changed to avoid unnecessary instance churn.
    let recurrence_changed;
    let new_recurrence = match (req.recurrence_rrule, req.recurrence_timezone) {
        // Both fields provided: validate and check for change.
        (Some(rrule), Some(tz)) => {
            // Build the candidate rule for validation and equality comparison.
            let candidate = RecurrenceRule::new(rrule, tz);
            match candidate.validate() {
                Ok(()) => {
                    // Only trigger re-materialisation if the rule actually differs
                    // from the current one. An identical PATCH must be a no-op.
                    recurrence_changed = task.recurrence.as_ref() != Some(&candidate);
                    Some(candidate)
                }
                // Invalid rule: 422 before any database writes.
                Err(e) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(serde_json::json!({ "error": format!("recurrence: {e}") })),
                    )
                        .into_response();
                }
            }
        }
        // Neither field provided: recurrence is not being changed in this request.
        (None, None) => {
            recurrence_changed = false;
            task.recurrence.clone()
        }
        // One without the other: the pair must arrive together.
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "recurrence_rrule and recurrence_timezone must both be provided or both absent"
                })),
            )
                .into_response();
        }
    };
    // Apply the possibly-updated recurrence to the task struct.
    task.recurrence = new_recurrence;

    // Bump `updated_at` to reflect that the task was just modified.
    task.updated_at = OffsetDateTime::now_utc();

    // ── Persist the updated task ──────────────────────────────────────────────
    //
    // All fields have been validated; write the complete updated struct to the DB.
    if let Err(e) = update_task(&state.db, &task).await {
        tracing::error!(error = %e, "failed to update task");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // ── Re-materialise instances if recurrence changed ────────────────────────
    //
    // When the recurrence rule changes, previously-materialised future instances
    // belong to the old schedule and must be replaced (ADR-0002 §mid-flight).
    if recurrence_changed && let Some(ref rule) = task.recurrence {
        // The cutoff is `updated_at` — instances at or after this moment are "future".
        let now = task.updated_at;
        // Step 1: delete old future instances for this task.
        if let Err(e) = delete_future_instances(&state.db, task.id, now).await {
            tracing::warn!(task_id = %task.id, error = %e, "failed to delete old instances");
        }
        // Step 2: generate and store new instances from the updated rule.
        if let Err(e) = materialise_and_store_instances(&state, &task, rule, now).await {
            tracing::warn!(task_id = %task.id, error = %e, "re-materialisation after recurrence change failed");
        }
    }

    // Return 200 with the updated task as confirmation of the changes.
    Json(task_to_response(&task)).into_response()
}

/// `POST /api/v1/tasks/{id}/complete` — mark a task instance as done.
///
/// Flow:
///   1. Validate the UUID in the path.
///   2. Parse `instance_date` from the body (YYYY-MM-DD).
///   3. Build an immutable `CompletionLog` with `skipped=false`.
///   4. Write the status update and log entry via `mark_task_done`.
///   5. Return HTTP 200 with the new log entry.
///
/// Writes an immutable `CompletionLog` entry with `skipped=false` and sets
/// the task's status to `done`. The completion log persists even if the task
/// is later reopened or updated.
///
/// The `instance_date` field must be a YYYY-MM-DD string. For recurring tasks
/// this is the scheduled date from the `task_instances` row; for one-shot tasks
/// the caller should supply the task's `due_by` date, or today if absent.
///
/// Returns HTTP 422 for a malformed `instance_date`.
/// Returns HTTP 200 with the `CompletionLog` entry on success.
pub async fn complete_task(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<CompleteTaskRequest>,
) -> impl IntoResponse {
    // Validate the path UUID: malformed strings are 400, not 422.
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // Parse the instance date from the request body.
    // For one-shot tasks: the caller should supply the `due_by` date or today.
    // For recurring tasks: the scheduled date from `task_instances.scheduled_at`.
    let instance_date = match parse_date(&req.instance_date) {
        Ok(d) => d,
        // Unparseable YYYY-MM-DD string: 422 with the invalid value quoted.
        Err(msg) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response();
        }
    };

    // Build the immutable completion record.
    // `skipped=false` distinguishes this from a skip event in the audit log.
    // The notes field carries any context the member chose to add (e.g.
    // "did it together with Anna" or "used the new cleaning spray").
    let completion = CompletionLog::new(
        id,
        instance_date,
        // Placeholder member until the Member entity and authentication land.
        placeholder_member(),
        // The completion timestamp is the current wall-clock UTC time.
        OffsetDateTime::now_utc(),
        false, // not skipped — this is a genuine completion
        req.notes,
    );

    // Write the status update and completion log via `mark_task_done`.
    // Both the task status and the log entry are written in one call.
    // On failure, log the error and return 500 — do not leak storage details.
    if let Err(e) = mark_task_done(&state.db, id, &completion).await {
        tracing::error!(error = %e, "failed to mark task done");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Return 200 with the new log entry so the client can display it immediately
    // in the history view without a separate GET request.
    (
        StatusCode::OK,
        Json(completion_log_to_response(&completion)),
    )
        .into_response()
}

/// `POST /api/v1/tasks/{id}/skip` — mark a task instance as skipped.
///
/// Flow:
///   1. Validate the UUID in the path.
///   2. Parse `instance_date` from the body (YYYY-MM-DD).
///   3. Build an immutable `CompletionLog` with `skipped=true`.
///   4. Write the status update and log entry via `mark_task_skipped`.
///   5. Return HTTP 200 with the new log entry.
///
/// Writes an immutable `CompletionLog` entry with `skipped=true` and sets the
/// task's status to `skipped`. Skipping is a first-class completion event
/// (brief §8.2): it records that the household decided not to do this instance,
/// without creating a "debt" — the next instance arrives on its normal schedule.
///
/// The skip entry appears alongside genuine completions in the `/history` endpoint.
/// The client renders skip events differently (e.g. grey "skipped" chip vs green "done").
///
/// Returns HTTP 422 for a malformed `instance_date`.
/// Returns HTTP 200 with the `CompletionLog` entry on success.
pub async fn skip_task(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<CompleteTaskRequest>,
) -> impl IntoResponse {
    // Validate the path UUID.
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // Parse the instance date — same conventions as the complete endpoint.
    // For recurring tasks: the scheduled date from `task_instances.scheduled_at`.
    // For one-shot tasks: the task's `due_by` date or today if absent.
    let instance_date = match parse_date(&req.instance_date) {
        // Valid YYYY-MM-DD: proceed to build the skip record.
        Ok(d) => d,
        // Malformed date: 422 with the bad value quoted in the error.
        Err(msg) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response();
        }
    };

    // Build the skip log entry. `skipped=true` is the only difference from a
    // completion event; the schema otherwise treats them identically.
    // The notes field carries any context the member chose to add (e.g. "chimney
    // sweep cancelled — rescheduled to next month").
    let completion = CompletionLog::new(
        id,
        instance_date,
        // Placeholder member until the Member entity and authentication land.
        placeholder_member(),
        // The skip timestamp is the current wall-clock UTC time.
        OffsetDateTime::now_utc(),
        true, // this is a skip, not a completion
        req.notes,
    );

    // Write the status update and skip log atomically via `mark_task_skipped`.
    // On failure, log the error and return 500 — do not leak storage details.
    if let Err(e) = mark_task_skipped(&state.db, id, &completion).await {
        tracing::error!(error = %e, "failed to mark task skipped");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Return 200 with the skip log entry so the client can display it immediately
    // in the history view without a separate GET request.
    (
        StatusCode::OK,
        Json(completion_log_to_response(&completion)),
    )
        .into_response()
}

/// `POST /api/v1/tasks/{id}/assignee` — change the current default assignee.
///
/// Flow:
///   1. Validate the task UUID in the path.
///   2. Parse the `member_id` UUID from the body (or `null` to clear).
///   3. Load the full task from storage.
///   4. Update `current_assignee_id` and bump `updated_at`.
///   5. Persist via `update_task` and return the updated task.
///
/// This is the "one-tap reassignment" described in the brief: a single POST
/// updates `current_assignee_id` without requiring a full PATCH body. Send
/// `null` for `member_id` to clear the current assignee.
///
/// `current_assignee_id` is a default for the whole task series, not a binding
/// obligation — any household member can always do any task. The field is purely
/// an organisational hint (brief §8.2).
///
/// Returns HTTP 404 if the task does not exist.
/// Returns HTTP 422 for an invalid `member_id` UUID string.
/// Returns HTTP 200 with the updated task on success.
pub async fn change_assignee(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    Json(req): Json<ChangeAssigneeRequest>,
) -> impl IntoResponse {
    // Validate the task UUID from the URL path.
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // Parse the new assignee UUID, or treat `null` as "clear the assignee".
    let new_assignee = match req.member_id {
        // A UUID string was provided: validate it before storing.
        Some(ref s) => match s.parse::<MemberId>() {
            // Valid UUID: will be stored as the new `current_assignee_id`.
            Ok(m) => Some(m),
            // Invalid UUID string: 422 with a terse message.
            Err(_) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": "invalid member ID" })),
                )
                    .into_response();
            }
        },
        // `null` in JSON: clear the assignee field.
        None => None,
    };

    // Load the task so we can update a single field and re-write the full struct.
    let mut task = match fetch_task(&state.db, id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch task");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Apply the assignee change: None clears the field, Some(m) sets a new default.
    // This does not affect `assignee_ids` — it only updates the `current_assignee_id`.
    task.current_assignee_id = new_assignee;
    // Bump `updated_at` so clients can detect that the task changed.
    task.updated_at = OffsetDateTime::now_utc();

    // Persist the change via the standard full-struct update path.
    // The storage layer always writes all columns, so no partial-update complexity.
    if let Err(e) = update_task(&state.db, &task).await {
        tracing::error!(error = %e, "failed to update task assignee");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Return the updated task confirming the new assignee state.
    // HTTP 200 (not 204) so the client gets the refreshed task without a separate GET.
    Json(task_to_response(&task)).into_response()
}

/// `GET /api/v1/tasks/{id}/history` — list completion log entries for a task.
///
/// Returns the full audit history for this task: every completion and skip event,
/// ordered `completed_at DESC` (most recent first). An empty array is valid for
/// a task that has never been completed or skipped.
///
/// The history includes both `skipped=false` (completion) and `skipped=true` (skip)
/// events. The household can review the full picture — when was this last done, and
/// when was it skipped and why? The `notes` field carries member context if set.
///
/// Completion logs are immutable — neither this endpoint nor any other allows
/// modification. If a log was written in error, the household adds a corrective note
/// to the next entry rather than editing the existing one (brief §6.5).
///
/// Returns HTTP 400 for a malformed task UUID in the path.
/// Returns HTTP 200 with a JSON array (possibly empty) on success.
pub async fn get_task_history(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    // Validate the task UUID from the path.
    let Ok(id) = id_str.parse::<TaskId>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid task ID" })),
        )
            .into_response();
    };

    // Fetch all completion log entries for this task.
    // The storage layer orders them `completed_at DESC` automatically.
    match list_completion_logs_for_task(&state.db, id).await {
        Ok(logs) => {
            // Map each domain `CompletionLog` to its flat JSON representation.
            let responses: Vec<CompletionLogResponse> =
                logs.iter().map(completion_log_to_response).collect();
            // HTTP 200 with the array — an empty array is a valid response.
            Json(responses).into_response()
        }
        // Unexpected storage error: log and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list completion logs");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Convert a domain `Task` into its flat JSON response representation.
///
/// All `OffsetDateTime` values become RFC 3339 strings.
/// All `MemberId` values become hyphenated UUID strings.
/// Optional fields that are `None` are omitted from the JSON body
/// via the `#[serde(skip_serializing_if = "Option::is_none")]` attributes.
///
/// This helper is used by every endpoint that returns a `TaskResponse`:
/// create, get, patch, and change-assignee all produce the identical shape.
/// The client can use a single deserialiser for all of them.
fn task_to_response(task: &Task) -> TaskResponse {
    // Format the audit timestamps as RFC 3339.
    // `unwrap_or_default` returns an empty string on format error — a safety net
    // that cannot trigger in practice because stored datetimes are always valid.
    let created_at = task.created_at.format(&Rfc3339).unwrap_or_default();
    let updated_at = task.updated_at.format(&Rfc3339).unwrap_or_default();
    // Optional datetimes: format when present, produce None when absent.
    let due_by = task.due_by.and_then(|dt| dt.format(&Rfc3339).ok());
    let earliest_at = task.earliest_at.and_then(|dt| dt.format(&Rfc3339).ok());

    // Build the flat response struct. Each field is described by its doc comment
    // on the struct definition above; the comments here explain the conversion.
    TaskResponse {
        // `TaskId::Display` gives the canonical hyphenated UUID string form.
        id: task.id.to_string(),
        // Title and notes are cloned — `task` is borrowed, not consumed.
        title: task.title.clone(),
        notes: task.notes.clone(),
        // `TaskStatus::Display` gives the snake_case string (open, doing, done, skipped).
        status: task.status.to_string(),
        // Already formatted as RFC 3339; None when open-ended.
        due_by,
        // Already formatted as RFC 3339; None when task is immediately available.
        earliest_at,
        // Extract the raw `u8` value from the newtype wrappers.
        effort: task.effort.map(amity_core::task::EffortLevel::value),
        // Same extraction pattern as effort; None when not ranked.
        priority: task.priority.map(amity_core::task::Priority::value),
        // Tags are already normalised at write time; clone as-is.
        tags: task.tags.clone(),
        // Single UUID fields: convert via `to_string()`.
        owner_id: task.owner_id.to_string(),
        // Vec<MemberId> fields: convert each element to its UUID string.
        assignee_ids: task
            .assignee_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        eligible_member_ids: task
            .eligible_member_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        // Optional single member: map to String only when present.
        current_assignee_id: task.current_assignee_id.map(|m| m.to_string()),
        // Recurrence: extract the two stored strings from the option.
        recurrence_rrule: task.recurrence.as_ref().map(|r| r.rrule.clone()),
        // Always present together with `recurrence_rrule`; absent for one-shot tasks.
        recurrence_timezone: task.recurrence.as_ref().map(|r| r.timezone.clone()),
        // Immutable creation timestamp formatted as RFC 3339.
        created_at,
        // Last-modification timestamp; bumped on every update and completion.
        updated_at,
    }
}

/// Convert a `CompletionLog` entry to its flat JSON response representation.
///
/// Formats `completed_at` as RFC 3339 and `instance_date` as YYYY-MM-DD.
/// Used by the complete, skip, and history endpoints — all three return the
/// same `CompletionLogResponse` shape so the client needs only one deserialiser.
fn completion_log_to_response(log: &CompletionLog) -> CompletionLogResponse {
    // Format the precise completion timestamp as RFC 3339 (with offset).
    let completed_at = log.completed_at.format(&Rfc3339).unwrap_or_default();
    // Format the calendar date as YYYY-MM-DD. `time::Date` has no dedicated
    // `YYYY-MM-DD` formatter that avoids a format string; we construct it here.
    let instance_date = format!(
        "{:04}-{:02}-{:02}",
        log.instance_date.year(),
        // `month()` returns a `time::Month` enum; `as u8` gives the numeric value.
        log.instance_date.month() as u8,
        log.instance_date.day()
    );

    // Build the flat response. All fields are described by doc comments on the
    // `CompletionLogResponse` struct definition above.
    CompletionLogResponse {
        // UUID v7 string — time-ordered, globally unique identifier for this log entry.
        id: log.id.to_string(),
        // UUID string of the task that was acted on.
        task_id: log.task_id.to_string(),
        // YYYY-MM-DD string already constructed above.
        instance_date,
        // UUID string of the member who performed the completion or skip action.
        completed_by: log.completed_by.to_string(),
        // RFC 3339 string with UTC offset, already formatted above.
        completed_at,
        // Boolean `skipped` maps directly to JSON true/false.
        skipped: log.skipped,
        // Optional free-form note; None is omitted from the JSON body.
        notes: log.notes.clone(),
    }
}

/// Parse a `YYYY-MM-DD` string from a request body into a `time::Date`.
///
/// Returns a user-readable error message on failure, suitable for use as the
/// value of an `"error"` key in a HTTP 422 response body.
///
/// The date string comes from the client's `instance_date` field. For recurring
/// tasks the client should supply the date from the `task_instances.scheduled_at`
/// column (already truncated to date by the client). For one-shot tasks the client
/// supplies the task's `due_by` date (also truncated), or today's date if absent.
fn parse_date(s: &str) -> Result<time::Date, String> {
    // Build the ISO 8601 date format descriptor. This uses the `time` crate's
    // bracketed format syntax, which is different from strftime. The string is a
    // compile-time constant so the inner `parse` cannot fail at runtime.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant; parse never fails");
    // Attempt to parse the input. Map a parse error to a user-readable message
    // that quotes the invalid value for easier client-side debugging.
    // The error message intentionally quotes `s` so operators can spot bad input
    // in the logs without needing to inspect the raw HTTP request body.
    time::Date::parse(s, &fmt).map_err(|_| format!("instance_date '{s}': expected YYYY-MM-DD"))
}

/// Materialise and persist recurring task instances up to a 60-day horizon.
///
/// Called immediately after task creation and after a recurrence rule change.
/// The core materialiser (`amity_core::recurrence_materialiser`) computes the
/// scheduled datetimes. This function wraps the results into `TaskInstance`
/// structs and writes them via `upsert_task_instances` (idempotent).
///
/// The 60-day horizon is chosen so the hub always has at least two months of
/// upcoming instances pre-computed. The background job (not yet implemented)
/// extends the horizon daily to keep it rolling. If the background job lags,
/// the worst case is that instances beyond the current horizon are not shown
/// until the next job run.
///
/// The holiday skip set is empty at this stage — the holiday calendar source
/// is not yet wired in (ADR-0002 §future-work). All computed dates are kept.
///
/// Failures are returned as a `String` error message so the caller can log
/// the error and continue — a failed materialisation does not fail the request.
/// The task row itself is already committed; the background job will catch up.
async fn materialise_and_store_instances(
    state: &AppState,
    task: &Task,
    rule: &RecurrenceRule,
    from: OffsetDateTime,
) -> Result<(), String> {
    // Set the forward horizon to 60 days from the start time.
    // The background job (not yet implemented) extends this on a daily schedule.
    let horizon = from + time::Duration::days(60);

    // No holiday skip set at this stage — the holiday calendar source is not
    // yet wired in. The stub returns an empty `HashSet`, so all instances are kept.
    let skip_dates = std::collections::HashSet::new();

    // Call the core materialiser: RRULE + DTSTART + IANA timezone → scheduled datetimes.
    // Wall-clock time is preserved across DST transitions (08:00 stays 08:00).
    let datetimes = amity_core::recurrence_materialiser::materialise_instances(
        rule,
        from,
        horizon,
        &skip_dates,
    )
    .map_err(|e| format!("materialise_instances: {e}"))?;

    // Convert each `OffsetDateTime` to a `TaskInstance` struct for storage.
    let instances: Vec<TaskInstance> = datetimes
        .into_iter()
        .map(|scheduled_at| TaskInstance {
            // Generate a fresh UUID v7 for each instance row.
            // UUID v7 is time-ordered, which makes the table index-friendly.
            id: uuid::Uuid::now_v7().to_string(),
            // Link back to the parent recurring task.
            task_id: task.id,
            scheduled_at,
            // Inherit the parent task's default assignee for each new instance.
            // The user can override this per-instance via the assignee endpoint.
            current_assignee_id: task.current_assignee_id,
        })
        .collect();

    // Persist the instances via INSERT OR IGNORE.
    // If this function is called twice for the same time window (e.g. after a
    // concurrent materialisation run), duplicates are silently skipped because
    // the `(task_id, scheduled_at)` UNIQUE constraint is enforced at the DB level.
    upsert_task_instances(&state.db, &instances)
        .await
        .map_err(|e| format!("upsert_task_instances: {e}"))
}
