// api/surfacing.rs — HTTP handler for the surfacing (Today) query.
//
// Endpoint:
//   GET /api/v1/surfacing/today?date=YYYY-MM-DD  — the ranked "what's on today"
//
// This is the service-layer half of the surfacing feature. The pure ranking
// rule lives in `amity_core::surfacing`; this handler's job is to *assemble the
// candidates* from storage, hand them to the rule, and serialise the result:
//
//   1. Resolve the target day (?date=…, else today in UTC).
//   2. Load every task; one-shot tasks become a Window candidate, recurring
//      tasks contribute a Scheduled candidate per materialised instance on the
//      day. (The pure rule drops resolved/undated ones — we do not pre-filter.)
//   3. Rank via `rank_today` and return a uniform, type-tagged item list plus
//      the `has_surfaced` empty-state flag.
//
// The response shape is `SurfacedKind`-tagged so the Today view renders a
// mixed-type list from day one; only Task feeds it today (Event/Project/Thread
// are the seam). Overdue items carry a flag the frontend renders as plain
// information — never a lateness count (brief §3, §11).
//
// Error handling matches api/task.rs:
//   HTTP 400 Bad Request     — the `date` query parameter is not YYYY-MM-DD.
//   HTTP 500 Internal Error  — an unexpected storage failure (logged, not leaked).

// axum's JSON response wrapper.
use axum::Json;
// `Query` extracts `?date=…`; `State` injects the shared pool.
use axum::extract::{Query, State};
// HTTP status codes for the error responses.
use axum::http::StatusCode;
// Trait that lets tuples / `Json<T>` / `StatusCode` become responses.
use axum::response::IntoResponse;
// `Deserialize` for the query struct; `Serialize` for the response structs.
use serde::{Deserialize, Serialize};
// `json!` builds the small error bodies.
use serde_json::json;
// `Date` is the surfaced day; `OffsetDateTime` is `now`; `Duration` shifts the
// instance lower bound; `Rfc3339` formats the salient instant for the wire.
use time::format_description::well_known::Rfc3339;
use time::{Date, Duration, OffsetDateTime};

// The pure surfacing rule and its types.
use amity_core::surfacing::{
    Liveness, SurfaceCandidate, SurfacedItem, SurfacedKind, SurfacingConfig, Timing, rank_today,
};
// `Task` is what we assemble candidates from; `TaskStatus` maps onto `Liveness`.
use amity_core::task::{Task, TaskStatus};
// Storage: list all tasks, and list a recurring task's materialised instances.
use amity_storage::task::{TaskFilter, list_tasks};
use amity_storage::task_instance::list_upcoming_instances;

// `AppState` carries the shared `SqlitePool`.
use crate::AppState;

// ─── Query and response types ───────────────────────────────────────────────

/// Query parameters for `GET /api/v1/surfacing/today`.
///
/// `date` is optional. Absent means "today" computed from the server's UTC
/// clock. A proper household-local (Amsterdam) date depends on member timezone
/// preferences, which do not exist yet — passing `?date=` is the override until
/// then.
#[derive(Debug, Deserialize)]
pub struct TodayQuery {
    /// The day to surface, as `YYYY-MM-DD`; absent → today (UTC).
    pub date: Option<String>,
}

/// JSON shape of one surfaced item.
///
/// Uniform across entity types: `kind` names the source (only `"task"` today),
/// and the salient instant `at` plus the `overdue` flag are already resolved by
/// the ranking rule. Optional fields are omitted from the body when absent.
#[derive(Debug, Serialize)]
pub struct SurfacedItemResponse {
    /// The source entity type — `"task"` for now (the mixed-type seam).
    pub kind: String,
    /// UUID string of the source task (the parent, for a recurring instance).
    pub source_id: String,
    /// One-line title, shown verbatim.
    pub title: String,
    /// Salient instant (RFC 3339): the scheduled time, or the due/earliest time.
    pub at: String,
    /// True when the item surfaced because it is open past its deadline (tasks).
    pub overdue: bool,
    /// True for an all-day event; the client shows "all day" instead of a time.
    pub all_day: bool,
    /// Importance rank 1-5; omitted when not ranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// UUID string of the member shown as responsible; omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_assignee_id: Option<String>,
}

/// JSON envelope returned by the Today endpoint.
///
/// `has_surfaced` is the designed-empty-state signal: when `false` the hub shows
/// a calm "nothing today" rather than an error or a spinner (brief §3, §11.5).
#[derive(Debug, Serialize)]
pub struct TodayResponse {
    /// The day this response is for (`YYYY-MM-DD`), echoed for the client.
    pub date: String,
    /// Whether anything crossed the "surface today" threshold.
    pub has_surfaced: bool,
    /// The surfaced items, already ordered by the ranking rule.
    pub items: Vec<SurfacedItemResponse>,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// `GET /api/v1/surfacing/today` — the ranked "what's on today" query.
///
/// Assembles candidates from storage, ranks them with the pure rule, and returns
/// the ordered items plus the empty-state flag.
///
/// Returns HTTP 400 if `date` is present but not `YYYY-MM-DD`.
/// Returns HTTP 500 on an unexpected storage failure.
pub async fn today(
    State(state): State<AppState>,
    Query(params): Query<TodayQuery>,
) -> impl IntoResponse {
    // Capture `now` once — the overdue comparison and the default day share it.
    let now = OffsetDateTime::now_utc();

    // Resolve the target day: an explicit `?date=` wins; otherwise today (UTC).
    let target_date = match params.date.as_deref() {
        // A supplied date must parse as YYYY-MM-DD, else it is a client error.
        Some(s) => match parse_date(s) {
            Ok(d) => d,
            Err(msg) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response();
            }
        },
        // Absent: fall back to the server's current UTC date.
        None => now.date(),
    };

    // Load every task. The pure rule drops resolved (Done/Skipped) and undated
    // ones, so there is no need to pre-filter by status here.
    let tasks = match list_tasks(&state.db, &TaskFilter::default()).await {
        Ok(t) => t,
        // A failure to read tasks is unexpected — log it and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list tasks for surfacing");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Turn the tasks (and their instances) into ranking candidates.
    let candidates = build_candidates(&state, &tasks, target_date).await;

    // Rank the day. No member filter yet — the whole household's day surfaces.
    let result = rank_today(candidates, target_date, now, &SurfacingConfig::default());

    // Project the ranked items into the wire shape and assemble the envelope.
    // Ordering is already decided by the rule; we only reshape each item here.
    let items = result.items.iter().map(surfaced_item_to_response).collect();
    let body = TodayResponse {
        // Echo the resolved day so the client can label the view.
        date: format_date(target_date),
        // The empty-state signal the hub keys "nothing today" off.
        has_surfaced: result.has_surfaced,
        // The ranked, reshaped items.
        items,
    };
    // 200 OK with the envelope — surfacing has no "not found" case.
    Json(body).into_response()
}

// ─── Candidate assembly ─────────────────────────────────────────────────────

/// Build the ranking candidates for `target_date` from the task set.
///
/// One-shot tasks each contribute a single `Window` candidate. Recurring tasks
/// contribute a `Scheduled` candidate per materialised instance that lands on
/// the day. Denormalises the parent task's title/status/priority onto each
/// instance so the ranked items render without a second fetch.
async fn build_candidates(
    state: &AppState,
    tasks: &[Task],
    target_date: Date,
) -> Vec<SurfaceCandidate> {
    // Accumulate candidates across all tasks.
    let mut candidates: Vec<SurfaceCandidate> = Vec::new();

    // Lower bound for the per-task instance query. It sits two days before the
    // day so an instance scheduled at local midnight — which lands in the prior
    // UTC day for a positive offset — is still fetched, then filtered by date.
    let lower_bound = target_date.midnight().assume_utc() - Duration::days(2);

    // Each task becomes zero or more candidates depending on its shape.
    for task in tasks {
        // A one-shot task (no recurrence rule) has no materialised instances.
        if task.recurrence.is_none() {
            // ── One-shot task: a single window candidate ────────────────────
            // The rule decides whether today intersects [earliest_at, due_by].
            candidates.push(SurfaceCandidate {
                // A one-shot task candidate.
                kind: SurfacedKind::Task,
                // The task's own id — one-shot tasks have no separate instance.
                source_id: task.id.to_string(),
                // Title shown verbatim in the Today view.
                title: task.title.clone(),
                // Map the task's status onto the kind-agnostic liveness flag.
                liveness: liveness_of(task.status),
                // A one-shot carries its window straight into the rule.
                timing: Timing::Window {
                    // Earliest meaningful start, if any.
                    earliest_at: task.earliest_at,
                    // Latest due time, if any.
                    due_by: task.due_by,
                },
                // Tasks are never all-day.
                all_day: false,
                // Soft rank for the ordering tiebreak.
                priority: task.priority,
                // The member shown as responsible, if set.
                current_assignee_id: task.current_assignee_id,
            });
        } else {
            // ── Recurring task: one candidate per instance on the day ───────
            // Fetch a generous window of upcoming instances, then keep only
            // those whose scheduled day matches the target.
            let instances = match list_upcoming_instances(&state.db, task.id, lower_bound, 200)
                .await
            {
                Ok(i) => i,
                // A per-task read failure should not fail the whole request;
                // log it and move on so the rest of the day still surfaces.
                Err(e) => {
                    tracing::warn!(task_id = %task.id, error = %e, "failed to fetch instances for surfacing");
                    continue;
                }
            };
            for inst in instances {
                // Keep only the instances scheduled on the target day.
                if inst.scheduled_at.date() == target_date {
                    candidates.push(SurfaceCandidate {
                        // A recurring-task instance candidate.
                        kind: SurfacedKind::Task,
                        // The parent task's id, for client navigation and actions.
                        source_id: task.id.to_string(),
                        // Title denormalised from the parent so no second fetch is needed.
                        title: task.title.clone(),
                        // Parent status → liveness; a Done series is dropped by the rule.
                        liveness: liveness_of(task.status),
                        // A recurring instance surfaces on its scheduled instant.
                        timing: Timing::Scheduled(inst.scheduled_at),
                        // Tasks are never all-day.
                        all_day: false,
                        // Soft rank from the parent task.
                        priority: task.priority,
                        // Prefer the instance's assignee; fall back to the task's.
                        current_assignee_id: inst.current_assignee_id.or(task.current_assignee_id),
                    });
                }
            }
        }
    }

    // The rule sorts and filters; we just hand it the full candidate set.
    candidates
}

// ─── Response mapping and helpers ───────────────────────────────────────────

/// Project a ranked `SurfacedItem` into its JSON response shape.
fn surfaced_item_to_response(item: &SurfacedItem) -> SurfacedItemResponse {
    SurfacedItemResponse {
        // The kind string ("task" / "event"); the helper keeps it in one place.
        kind: kind_to_str(item.kind).to_owned(),
        // The id is already a string on the surfaced item.
        source_id: item.source_id.clone(),
        // Title is shown verbatim.
        title: item.title.clone(),
        // RFC 3339 for the salient instant; formatting a valid datetime never fails.
        at: item.at.format(&Rfc3339).unwrap_or_default(),
        // Carry the overdue flag straight through.
        overdue: item.overdue,
        // All-day flag drives the "all day" label and the lead-the-day ordering.
        all_day: item.all_day,
        // Priority newtype → its inner 1-5 value, or omitted when unset.
        priority: item.priority.map(amity_core::task::Priority::value),
        // Assignee UUID string, or omitted when unset.
        current_assignee_id: item.current_assignee_id.map(|m| m.to_string()),
    }
}

/// Map a `SurfacedKind` to its wire string.
fn kind_to_str(kind: SurfacedKind) -> &'static str {
    match kind {
        // Household task or task instance.
        SurfacedKind::Task => "task",
        // Calendar event (Project/Thread join here later).
        SurfacedKind::Event => "event",
    }
}

/// Map a task's lifecycle status onto the kind-agnostic surfacing liveness.
///
/// Open and Doing are live and can surface; Done and Skipped are settled and
/// never surface. Events do not use this — they are always live until passed.
fn liveness_of(status: TaskStatus) -> Liveness {
    match status {
        // Still actionable → eligible to surface.
        TaskStatus::Open | TaskStatus::Doing => Liveness::Live,
        // Resolved → filtered out by the rule.
        TaskStatus::Done | TaskStatus::Skipped => Liveness::Settled,
    }
}

/// Parse a `YYYY-MM-DD` string into a `time::Date`.
///
/// Mirrors the date parsing in api/task.rs so the two endpoints accept the same
/// format and produce the same style of error message.
fn parse_date(s: &str) -> Result<Date, String> {
    // The bracketed format is a compile-time constant, so building it never fails.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant");
    // Quote the bad value in the error so it is easy to spot in logs.
    Date::parse(s, &fmt).map_err(|_| format!("date '{s}': expected YYYY-MM-DD"))
}

/// Format a `time::Date` as `YYYY-MM-DD` for the response envelope.
fn format_date(date: Date) -> String {
    // Same constant format as `parse_date`; formatting a valid date never fails.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant");
    date.format(&fmt)
        .expect("formatting a valid date never fails")
}
