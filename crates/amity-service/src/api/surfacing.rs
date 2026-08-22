// api/surfacing.rs — HTTP handlers for the surfacing (Today) and Week queries.
//
// Endpoints:
//   GET /api/v1/surfacing/today?date=YYYY-MM-DD   — the ranked "what's on today"
//   GET /api/v1/week?start=YYYY-MM-DD             — the 7-day Monday-start layout
//
// This is the service-layer half of the surfacing feature. The pure ranking
// rule (`rank_today`) and the pure layout rule (`plan_week`) both live in
// `amity_core::surfacing`; this file's job is to *assemble the candidates*
// from storage, hand them to whichever rule applies, and serialise the result:
//
//   1. Resolve the target day (?date=…, else today in UTC) — or, for Week, the
//      Monday of the target day's week (?start=…, else this week).
//   2. Load tasks AND events. One-shot tasks become a Window candidate; recurring
//      tasks a Scheduled candidate per instance; a one-shot NATIVE event a
//      Scheduled candidate on its start date; every other event — native
//      recurring, or any Ics event at all — one Scheduled candidate per
//      materialised `event_instances` row, minus any instance with a Cancel
//      override. (The pure rules drop resolved/undated tasks — we do not
//      pre-filter.) Week calls the SAME per-day builders once for each of the
//      7 days and concatenates the results — see `week`'s doc comment for why
//      that is safe despite one-shot tasks not being day-scoped internally.
//   3. Rank/lay out via `rank_today`/`plan_week` and return a uniform,
//      type-tagged item list (Today) or 7 day-buckets of the same (Week), plus
//      Today's `has_surfaced` empty-state flag.
//
// The response shape is `SurfacedKind`-tagged so both views render a
// mixed-type list. Task and Event both feed it now; Project/Thread are the
// remaining seam. Overdue items carry a flag the frontend renders as plain
// information — never a lateness count (brief §3, §11).
//
// P2 Slice 3 adds a THIRD kind, Meal, but only to Today: `build_meal_candidates`
// turns a dinner-slot planned meal into one all-day, non-actionable candidate
// (see its own doc comment). `week`'s candidate assembly is deliberately left
// untouched — a planned meal never appears on the Week grid in this slice.
//
// Error handling matches api/task.rs:
//   HTTP 400 Bad Request     — the `date`/`start` query parameter is not YYYY-MM-DD.
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

// The pure surfacing rule and its types, plus Week's pure layout rule.
use amity_core::surfacing::{
    Liveness, SurfaceCandidate, SurfacedItem, SurfacedKind, SurfacingConfig, Timing, plan_week,
    rank_today,
};
// `Task` is what we assemble candidates from; `TaskStatus` maps onto `Liveness`.
use amity_core::task::{Task, TaskStatus};
// `MealSlot` filters Today's meal candidates down to the dinner slot.
use amity_core::meal::MealSlot;
// Storage: tasks and their instances; events, their instances, and overrides;
// meals for the Today-only meal candidate.
use amity_storage::event::list_events;
use amity_storage::event_instance::list_upcoming_event_instances;
use amity_storage::event_override::list_overrides_on_date;
use amity_storage::meal::list_meals_in_range;
use amity_storage::task::{TaskFilter, list_tasks};
use amity_storage::task_instance::list_upcoming_instances;
// Event override action/type (to detect cancellations and apply the rest) and
// the event id type used to key the day's overrides.
use amity_core::event_override::{EventOverride, OverrideAction};
use amity_core::ids::EventId;
// `EventSourceKind` distinguishes native from external (Ics) events — external
// events always route through the instance path (see `build_event_candidates`)
// because `Event.recurrence` is deliberately left `None` for them even when
// the source recurs (external recurrence is trusted to the feed, never
// mirrored onto the native recurrence field — see `amity_core::event`'s doc
// comment and `jobs::calendar_sync::build_events`).
use amity_core::event::EventSourceKind;
// A set of cancelled event ids for the day, and a lookup from event id to the
// day's applicable (non-cancel) override.
use std::collections::{HashMap, HashSet};

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
/// Uniform across entity types: `kind` names the source (`"task"` or `"event"`),
/// and the salient instant `at` plus the `overdue` and `all_day` flags are
/// already resolved by the ranking rule. Optional fields (priority, assignee)
/// are omitted from the body when absent.
#[derive(Debug, Serialize)]
pub struct SurfacedItemResponse {
    /// The source entity type — `"task"` or `"event"` (the mixed-type seam).
    pub kind: String,
    /// UUID string of the source task or event (the parent, for a recurring
    /// instance) — used for client navigation and the mark-done/override actions.
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
    /// A household note from an `Annotate` override; omitted when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    /// True when a `Reschedule` override moved this instance to a new time.
    pub rescheduled: bool,
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

/// Query parameters for `GET /api/v1/week`.
///
/// `start` is optional and need not itself be a Monday — any date inside the
/// target week works, mirroring how `?date=` on Today accepts any single day.
/// Absent means "this week", computed from the server's UTC clock.
#[derive(Debug, Deserialize)]
pub struct WeekQuery {
    /// Any date inside the target week, as `YYYY-MM-DD`; absent → this week (UTC).
    pub start: Option<String>,
}

/// JSON shape of one day's bucket in a `WeekResponse`.
#[derive(Debug, Serialize)]
pub struct WeekDayResponse {
    /// The calendar date this bucket is for (`YYYY-MM-DD`).
    pub date: String,
    /// The items placed on this day, already ordered by `plan_week`'s layout
    /// rule (all-day first, then events before tasks, then by salient time).
    pub items: Vec<SurfacedItemResponse>,
}

/// JSON envelope returned by `GET /api/v1/week`.
///
/// Always exactly 7 `days`, Monday-first — Week has no empty-state flag the
/// way Today does, because an empty grid is still a complete, meaningful
/// answer (a quiet week), not a "nothing to show" case needing a signal.
#[derive(Debug, Serialize)]
pub struct WeekResponse {
    /// The Monday this week starts on (`YYYY-MM-DD`), echoed for the client.
    pub start: String,
    /// Exactly 7 day buckets, `start` through `start + 6 days`, in order.
    pub days: Vec<WeekDayResponse>,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// `GET /api/v1/surfacing/today` — the ranked "what's on today" query.
///
/// Assembles task and event candidates from storage, ranks them with the pure
/// rule, and returns the ordered mixed-type items plus the empty-state flag.
/// This is the one endpoint the Today view calls; everything a household member
/// sees at rest flows through here.
///
/// A task-list read failure is fatal (500); an event read failure is not — the
/// event contribution degrades to empty so the day still shows its tasks.
///
/// Returns HTTP 400 if `date` is present but not `YYYY-MM-DD`.
/// Returns HTTP 500 on an unexpected task-storage failure.
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
    // ones, so there is no need to pre-filter by status here. Unlike events, a
    // task-list read failure is fatal to the request.
    let tasks = match list_tasks(&state.db, &TaskFilter::default()).await {
        Ok(t) => t,
        // A failure to read tasks is unexpected — log it and return 500.
        Err(e) => {
            tracing::error!(error = %e, "failed to list tasks for surfacing");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Turn tasks, events, and meals into one mixed-type candidate set. All
    // three kinds flow through the same ranking rule; events and meals simply
    // carry different timing/all_day shapes. The three builders are
    // independent, so the order they are appended does not matter — the rule
    // re-sorts the whole set. Meals are Today-only — `week` (below) never
    // calls `build_meal_candidates` (see that function's doc comment).
    let mut candidates = build_task_candidates(&state, &tasks, target_date).await;
    candidates.extend(build_event_candidates(&state, target_date).await);
    candidates.extend(build_meal_candidates(&state, target_date).await);

    // Rank the whole mixed set at once. No member filter yet — the whole
    // household's day surfaces; per-person filtering arrives with members.
    let result = rank_today(candidates, target_date, now, &SurfacingConfig::default());

    // Project the ranked items into the wire shape and assemble the envelope.
    // Ordering is already decided by the rule; we only reshape each item here.
    // Tasks and events share the SurfacedItemResponse shape, so no per-kind
    // branching is needed at the wire boundary.
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

/// `GET /api/v1/week` — the Monday-start 7-day layout query.
///
/// Resolves `?start=` (any date inside the target week; absent → this week)
/// to that week's Monday, then builds the SAME per-day candidate set `today`
/// does — once for each of the 7 days — before handing the concatenated whole
/// to `plan_week`. This is deliberately not a 7x call to `today`'s handler:
/// `plan_week` needs the *union* of every day's candidates up front so it can
/// place recurring events/tasks on the day their own instance actually falls
/// on, in one pass, with one consistent `now`.
///
/// One-shot tasks are NOT filtered by day inside `build_task_candidates`
/// (Today only ever calls it once, so it does not need to be), so calling it
/// once per day yields 7 identical copies of the same one-shot task's
/// candidate. This is safe: `plan_week` collapses duplicate placements of the
/// same instance (same kind/source id/salient time) — see its doc comment.
///
/// Returns HTTP 400 if `start` is present but not `YYYY-MM-DD`.
/// Returns HTTP 500 on an unexpected task-storage failure.
pub async fn week(
    State(state): State<AppState>,
    Query(params): Query<WeekQuery>,
) -> impl IntoResponse {
    // Capture `now` once — shared by every day's overdue comparison.
    let now = OffsetDateTime::now_utc();

    // Resolve any date inside the target week: an explicit `?start=` wins;
    // otherwise today (UTC). Errors mirror `today`'s `?date=` handling.
    let anchor_date = match params.start.as_deref() {
        Some(s) => match parse_date(s) {
            Ok(d) => d,
            Err(msg) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response();
            }
        },
        None => now.date(),
    };

    // The Monday of `anchor_date`'s week: step back by its weekday offset
    // from Monday (0 for Monday itself, up to 6 for Sunday).
    let week_start =
        anchor_date - Duration::days(i64::from(anchor_date.weekday().number_days_from_monday()));

    // Load every task once; the per-day builder below is called once per day
    // but does not re-read the task list each time. As on Today, a task-list
    // read failure is fatal — an event read failure degrades gracefully.
    let tasks = match list_tasks(&state.db, &TaskFilter::default()).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to list tasks for week");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Build and concatenate all 7 days' candidates via the SAME per-day
    // builders Today uses — this carries overrides and external-event
    // instances automatically, with no separate Week-specific query path.
    let mut candidates: Vec<SurfaceCandidate> = Vec::new();
    for offset in 0_i64..7 {
        let day = week_start + Duration::days(offset);
        candidates.extend(build_task_candidates(&state, &tasks, day).await);
        candidates.extend(build_event_candidates(&state, day).await);
    }

    // Lay out the week. No member filter yet, matching Today.
    let plan = plan_week(candidates, week_start, now, &SurfacingConfig::default());

    // Project each day's ranked items into the wire shape.
    let days = plan
        .days
        .into_iter()
        .map(|day| WeekDayResponse {
            date: format_date(day.date),
            items: day.items.iter().map(surfaced_item_to_response).collect(),
        })
        .collect();
    let body = WeekResponse {
        // Echo the resolved Monday so the client can label the grid.
        start: format_date(plan.start),
        days,
    };
    // 200 OK with the envelope — Week, like Today, has no "not found" case.
    Json(body).into_response()
}

// ─── Candidate assembly ─────────────────────────────────────────────────────

/// Build the task ranking candidates for `target_date`.
///
/// The task half of the candidate set (events are gathered separately by
/// `build_event_candidates`). One-shot tasks each contribute a single `Window`
/// candidate. Recurring tasks
/// contribute a `Scheduled` candidate per materialised instance that lands on
/// the day. Denormalises the parent task's title/status/priority onto each
/// instance so the ranked items render without a second fetch.
async fn build_task_candidates(
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
                // Overrides target events only; tasks never carry one.
                annotation: None,
                rescheduled: false,
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
                        // Overrides target events only; tasks never carry one.
                        annotation: None,
                        rescheduled: false,
                    });
                }
            }
        }
    }

    // The rule sorts and filters; we just hand it the full candidate set.
    candidates
}

/// Build the event ranking candidates for `target_date`.
///
/// A one-shot NATIVE event contributes a candidate on its start date. Every
/// other event — a native recurring event, or ANY Ics-sourced event whether
/// or not it recurs — contributes one candidate per materialised
/// `event_instances` row on the day (see the `is_native_one_shot` branch
/// below for why Ics events always take this path). Any event with a
/// `Cancel` override for the day is dropped entirely. Events use `Scheduled`
/// timing (they surface on their day and are never overdue) and carry the
/// all-day flag through so all-day events lead the day.
///
/// All three override actions are applied here. `Cancel` drops the instance
/// entirely (see `cancelled`, below). `Reschedule` and `Annotate` are applied
/// per instance via the `applied` lookup keyed by `source_event_id`: a
/// `Reschedule` replaces the candidate's salient time with the payload's
/// parsed instant and sets `rescheduled`; an `Annotate` attaches the payload
/// as `annotation` and leaves timing untouched. Because each event
/// contributes at most one candidate for `target_date` (the one-shot branch
/// picks `start_at`; the instance branch keeps only the day's matching row),
/// replacing that candidate's time in place is enough to guarantee the
/// original slot never also surfaces — there is only ever one row to begin
/// with. A malformed `Reschedule` payload logs a warning and falls back to
/// the original time rather than panicking.
///
/// A storage read failure degrades to an empty event contribution rather than
/// failing the whole surfacing query, so tasks still surface if events cannot
/// be loaded — the Today view should never go blank over a partial failure.
async fn build_event_candidates(state: &AppState, target_date: Date) -> Vec<SurfaceCandidate> {
    // Load the full event set; a read failure yields an empty day rather than a
    // 500 for the whole surfacing query (tasks may still have surfaced).
    let events = match list_events(&state.db).await {
        // Got the events.
        Ok(e) => e,
        // A read failure here degrades gracefully to no events.
        Err(e) => {
            tracing::error!(error = %e, "failed to list events for surfacing");
            return Vec::new();
        }
    };

    // Load the day's overrides once. A read failure degrades to "no overrides"
    // rather than failing the request — worst case a cancelled event still shows.
    let overrides = list_overrides_on_date(&state.db, target_date)
        .await
        .unwrap_or_default();
    // A Cancel override for the day hides that event's instance entirely; collect
    // the cancelled event ids so the loop below can skip them in one lookup.
    let cancelled: HashSet<EventId> = overrides
        .iter()
        .filter(|o| o.action == OverrideAction::Cancel)
        .map(|o| o.source_event_id)
        .collect();
    // The day's Reschedule/Annotate overrides, keyed by the event they target.
    // An event has at most one override per instance_date in normal use, but
    // overlays are append-only (no update/delete), so if more than one was
    // ever recorded for the same instance, the most recently created one wins
    // — "a change of mind is a new overlay" (event_override.rs's doc comment).
    let mut applied: HashMap<EventId, &EventOverride> = HashMap::new();
    for o in &overrides {
        // Cancel is handled separately above; skip it here.
        if o.action == OverrideAction::Cancel {
            continue;
        }
        applied
            .entry(o.source_event_id)
            .and_modify(|existing| {
                // Keep whichever overlay was created last.
                if o.created_at > existing.created_at {
                    *existing = o;
                }
            })
            .or_insert(o);
    }

    // Lower bound for the per-event instance query, mirroring the task path.
    // Two days back so a local-midnight instance that lands in the prior UTC day
    // (for a positive offset) is still fetched, then filtered by date — the same
    // trick the task path uses.
    let lower_bound = target_date.midnight().assume_utc() - Duration::days(2);

    // Accumulate event candidates across the whole event set.
    // Accumulate one or more candidates per surviving event.
    let mut candidates: Vec<SurfaceCandidate> = Vec::new();
    for event in &events {
        // A cancel override for the day hides the event's instance entirely, so
        // skip the whole event before building any candidate for it.
        if cancelled.contains(&event.id) {
            continue;
        }
        // Route through the instance path whenever `event_instances` rows
        // actually exist for this event: that is true for any event with a
        // native recurrence rule, AND for every Ics-sourced event regardless
        // of whether it recurs — `calendar_sync::materialise_instances`
        // writes one instance row even for a one-shot external event (see
        // `expand_external`'s no-RRULE branch), so an Ics event always has at
        // least its single occurrence materialised. Without this, a
        // recurring external event's `Event.recurrence` is `None` (by
        // design — external recurrence is never copied onto it), so it would
        // wrongly fall into the one-shot `start_at` branch below and never
        // surface past its very first occurrence.
        let is_native_one_shot =
            event.recurrence.is_none() && event.source.kind != EventSourceKind::Ics;
        if is_native_one_shot {
            // One-shot NATIVE event: surfaces if its start instant falls on
            // the day. Native one-shot events have no materialised instance
            // rows, so `start_at` is the only source of truth for them.
            if event.start_at.date() == target_date {
                candidates.push(event_candidate(
                    // The event and its default (un-overridden) start time.
                    event,
                    event.start_at,
                    // A native one-shot event can still carry a Reschedule or
                    // Annotate for its single occurrence.
                    applied.get(&event.id).copied(),
                ));
            }
        } else {
            // Native recurring event, OR any Ics event (one-shot or
            // recurring): one candidate per materialised instance on the day.
            // Fetch a generous window, then keep only the target-day instances.
            let instances = match list_upcoming_event_instances(
                &state.db,
                event.id,
                lower_bound,
                200,
            )
            .await
            {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(event_id = %event.id, error = %e, "failed to fetch event instances");
                    continue;
                }
            };
            // Keep only the instances that land on the target day.
            for inst in instances {
                if inst.scheduled_at.date() == target_date {
                    candidates.push(event_candidate(
                        // The event and this materialised instance's time.
                        event,
                        inst.scheduled_at,
                        // The same per-event override lookup used above —
                        // this is the seam that also covers external events.
                        applied.get(&event.id).copied(),
                    ));
                }
            }
        }
    }

    // The pure rule filters and orders; we hand it the whole event candidate set.
    candidates
}

/// Build an Event-kind `SurfaceCandidate` at the given salient instant,
/// applying `override_for_event` (this event's Reschedule/Annotate overlay for
/// the day, if any — `None` means no non-Cancel override targets it today).
///
/// `at` is the event's start (one-shot) or an instance's scheduled time
/// (recurring); it is the instant that lands on `target_date`, so a
/// `Reschedule` payload is expected to name a time on that same day (Slice 1
/// does not support a cross-day reschedule — see `build_event_candidates`'s
/// caller-level doc). Events carry no priority or assignee and are never
/// overdue, so the rule orders them purely by time (all-day first).
fn event_candidate(
    event: &amity_core::event::Event,
    at: OffsetDateTime,
    override_for_event: Option<&EventOverride>,
) -> SurfaceCandidate {
    // Defaults for an un-overridden instance; each branch below may replace
    // the time and/or set the annotation.
    // The salient time, unless a Reschedule below replaces it.
    let mut surfaced_at = at;
    // Flips true only when a Reschedule payload parses successfully.
    let mut rescheduled = false;
    // Set only by an Annotate override; stays `None` otherwise.
    let mut annotation = None;

    if let Some(overlay) = override_for_event {
        // Dispatch on the overlay's action; Cancel cannot reach here (its
        // events are filtered out before any candidate is built).
        match overlay.action {
            // Reparse the payload as the new instant; a malformed value is
            // logged and the original time is kept rather than panicking.
            OverrideAction::Reschedule => match overlay
                .payload
                .as_deref()
                .map(|p| OffsetDateTime::parse(p, &Rfc3339))
            {
                Some(Ok(new_at)) => {
                    surfaced_at = new_at;
                    rescheduled = true;
                }
                Some(Err(e)) => {
                    tracing::warn!(
                        event_id = %event.id,
                        payload = ?overlay.payload,
                        error = %e,
                        "malformed reschedule payload; keeping original time"
                    );
                }
                // A Reschedule with no payload at all is equally malformed.
                None => {
                    tracing::warn!(
                        event_id = %event.id,
                        "reschedule override has no payload; keeping original time"
                    );
                }
            },
            // The note passes straight through; timing is untouched.
            OverrideAction::Annotate => annotation.clone_from(&overlay.payload),
            // Cancel never reaches this function (its events are skipped
            // before any candidate is built) — nothing to apply here.
            OverrideAction::Cancel => {}
        }
    }

    SurfaceCandidate {
        // Events are the second surfacable kind (the seam made real).
        kind: SurfacedKind::Event,
        // The event's id, for client navigation and the override action.
        source_id: event.id.to_string(),
        // Title shown verbatim in the Today view.
        title: event.title.clone(),
        // Events are always live until they have passed; there is no Done state.
        liveness: Liveness::Live,
        // A fixed occurrence: surfaces on its day, never overdue. `surfaced_at`
        // is either the original instant or the Reschedule's new one.
        timing: Timing::Scheduled(surfaced_at),
        // All-day events lead the day; timed ones sort by time.
        all_day: event.all_day,
        // Events carry no priority and no assignee.
        priority: None,
        current_assignee_id: None,
        // Set only by an Annotate override targeting this instance.
        annotation,
        // Set only by a successfully-parsed Reschedule override.
        rescheduled,
    }
}

/// Build Today's Meal candidates for `target_date` (P2 Slice 3).
///
/// Only `today` calls this — `week` is deliberately untouched, per the P2
/// Slice 3 brief, so a planned meal never appears on the Week grid. A
/// dinner-slot `Meal` planned for the day becomes exactly ONE informational,
/// non-actionable `SurfaceCandidate`: `all_day: true` (meals carry no clock
/// time, so they lead the day like an all-day event — see `rank_today`'s
/// sort), anchored at local midnight so the existing `Scheduled` timing rule
/// places it on its own day with no new rule needed, and `cook` mapped
/// straight onto `current_assignee_id` (the same responsible-person field
/// tasks and events already use). Breakfast/lunch/other-slot meals do not
/// surface here — the brief scopes Today's meal surfacing to dinner, the
/// slot households overwhelmingly plan ahead of time (see
/// `amity_core::meal::MealSlot`'s doc comment).
///
/// A storage read failure degrades to an empty meal contribution, matching
/// `build_event_candidates`'s own graceful degradation — a meals outage
/// should not blank out the rest of the day.
async fn build_meal_candidates(state: &AppState, target_date: Date) -> Vec<SurfaceCandidate> {
    // A single-day range: `from == to == target_date`. Reuses the grocery
    // generator's own storage query rather than a bespoke one-day fetch.
    let meals = match list_meals_in_range(&state.db, target_date, target_date).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list meals for surfacing");
            return Vec::new();
        }
    };

    // Local midnight on the meal's own date — no meal carries a clock time,
    // so this is the anchor `Timing::Scheduled` needs to place it on exactly
    // `target_date` and nowhere else.
    let anchor = target_date.midnight().assume_utc();

    meals
        .into_iter()
        // Today's meal surfacing is scoped to the dinner slot (see the doc
        // comment above).
        .filter(|m| m.slot == MealSlot::Dinner)
        .map(|meal| SurfaceCandidate {
            // The third surfacable kind (P2 Slice 3's addition to the seam).
            kind: SurfacedKind::Meal,
            // The meal's own id, for client navigation.
            source_id: meal.id.to_string(),
            // Title shown verbatim in the Today view.
            title: meal.name,
            // Meals have no lifecycle to settle (no Done/Skipped) — always
            // live, so only the temporal rule decides whether it surfaces.
            liveness: Liveness::Live,
            // Fixed occurrence on the meal's own date; never overdue.
            timing: Timing::Scheduled(anchor),
            // No clock time — leads the day like a banner event.
            all_day: true,
            // Meals carry no soft-priority ranking.
            priority: None,
            // Reuse the existing responsible-person field for the cook.
            current_assignee_id: meal.cook,
            // No override machinery for meals in this slice.
            annotation: None,
            rescheduled: false,
        })
        .collect()
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
        // Carry the override-derived fields straight through to the wire.
        annotation: item.annotation.clone(),
        rescheduled: item.rescheduled,
    }
}

/// Map a `SurfacedKind` to its wire string.
fn kind_to_str(kind: SurfacedKind) -> &'static str {
    match kind {
        // Household task or task instance.
        SurfacedKind::Task => "task",
        // Calendar event (Project/Thread join here later).
        SurfacedKind::Event => "event",
        // A planned dinner meal, surfaced on Today only (P2 Slice 3).
        SurfacedKind::Meal => "meal",
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
