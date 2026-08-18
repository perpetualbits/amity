// jobs/calendar_sync.rs — the ICS calendar sync job.
//
// Amity is a calendar aggregator (brief §7): the household subscribes to
// read-only external ICS feeds (school, club, waste, holiday, personal), and
// this job is what actually keeps those feeds fresh. On each run it, for
// every *enabled* calendar:
//   1. fetches the feed's raw text (the only network egress in Amity — see
//      `crate::feeds`);
//   2. parses it into `ParsedEvent`s (`amity_core::ics::parse_feed`, pure,
//      network-free);
//   3. upserts the corresponding `Event` rows, keyed by (calendar, UID);
//   4. prunes any previously-ingested event whose UID vanished from the feed;
//   5. re-materialises each event's instances out to the 60-day horizon
//      (matching the horizon used for native events/tasks), first sweeping
//      that event's not-yet-happened instance rows so a rescheduled DTSTART
//      or a newly-added EXDATE does not leave stale rows behind (see
//      `materialise_instances`'s doc comment); and
//   6. records the outcome (`Ok` / `Unreachable` / `ParseError`) on the
//      calendar's sync state.
//
// A fetch or parse failure is NOT fatal to the calendar's existing data: the
// brief requires that a temporarily-unreachable or briefly-broken feed keep
// showing its last-known-good events rather than going blank. Only a genuine
// storage-layer failure (a database problem, not a feed problem) propagates
// as an `Err` from `sync_one` — see its doc comment for the precise split.
//
// `run_once` holds the testable orchestration logic and takes an explicit
// `now` plus an injected `fetch` function, so tests can drive it against
// fixture ICS text with no real network call (see `tests/calendar_sync.rs`).
// `spawn` is the thin wrapper that runs it on startup and then every 6 hours,
// passing the real `feeds::fetch` as the injected function — mirroring
// `jobs::recurrence_horizon` (see that module for the pattern this follows).

// The shared connection pool.
use sqlx::SqlitePool;
// `Future` names the associated type the injected `fetch` function returns;
// `Duration`/`OffsetDateTime` drive the horizon arithmetic.
use std::future::Future;
use time::{Duration, OffsetDateTime};

// The egress module: `FetchError` is threaded through the generic `fetch`
// parameter below; `fetch` itself is only referenced by `spawn`.
use crate::feeds::{self, FetchError};

// Pure ICS parsing/expansion — no I/O, fully unit-tested on its own.
use amity_core::ics::{ParsedEvent, expand_external, parse_feed};
// The domain types a parsed feed becomes.
use amity_core::event::{Event, EventBuilder, EventSource};
use amity_core::ids::EventId;
// Calendar read/write: load the subscription list and record sync outcomes.
use amity_core::calendar::{CalendarSyncState, SyncStatus};
use amity_storage::calendar::{StoredCalendar, list_calendars, update_calendar_sync_state};
// Event persistence: upsert the ingested rows and prune the ones that vanished.
use amity_storage::event::{list_events, prune_events_missing_from_feed, upsert_external_events};
// Instance persistence: the materialised occurrences the Today view reads.
// `delete_future_event_instances` sweeps an event's not-yet-happened instance
// rows before re-materialising — see `materialise_instances`'s doc comment for
// why a re-sync needs this (Important #2, final whole-branch review).
use amity_storage::event_instance::{
    EventInstance, delete_future_event_instances, upsert_event_instances,
};

// ─── Tuning constants ───────────────────────────────────────────────────────

/// How far forward event instances are materialised, in days.
///
/// Matches the 60-day horizon used for native events and tasks (see
/// `jobs::recurrence_horizon::HORIZON_DAYS`), so an external event and a
/// native one present the same forward window on the Today view.
const HORIZON_DAYS: i64 = 60;

/// How often the spawned job runs, in seconds (6 hours).
///
/// Shorter than the 24-hour recurrence-horizon interval: external feeds are
/// edited outside Amity's control (a school adds a snow day, a club moves a
/// practice), so the household should see those changes within a few hours,
/// not a full day.
const RUN_INTERVAL_SECS: u64 = 6 * 60 * 60;

// ─── Report ─────────────────────────────────────────────────────────────────

/// What one sync pass (across every enabled calendar) did — for logging and
/// for tests to assert on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Enabled calendars whose sync attempt completed without a storage-level
    /// error (this counts an `Unreachable`/`ParseError` outcome too — those
    /// are recorded feed problems, not job failures; see `sync_one`).
    pub calendars_synced: usize,
    /// Total events handed to `upsert_external_events`, summed across every
    /// calendar synced this pass.
    pub events_upserted: usize,
    /// Total events deleted because they vanished from their feed, summed
    /// across every calendar synced this pass.
    pub events_pruned: u64,
}

/// The two numbers a single calendar's sync produces, beyond the pass/fail
/// outcome already recorded on the calendar's own sync state.
///
/// Kept separate from [`sync_one`]'s public `Result<usize, String>` return so
/// `run_once` can accumulate an accurate [`SyncReport`] (including the prune
/// count) without a second pass over the same calendar. `sync_one` itself
/// only surfaces `events_upserted` — the number a "sync this feed now" UI
/// action would want to show — matching the interface the next task
/// (the calendars API) is written against.
struct SyncOutcome {
    /// Events written by `upsert_external_events` this pass (0 on a fetch or
    /// parse failure, since nothing was ingested).
    events_upserted: usize,
    /// Events removed by `prune_events_missing_from_feed` this pass (0 on a
    /// fetch or parse failure, since pruning never runs in that case).
    events_pruned: u64,
}

// ─── One calendar's sync ────────────────────────────────────────────────────

/// Sync a single calendar and return how many events it produced.
///
/// A fetch failure records [`SyncStatus::Unreachable`] (keeping the
/// calendar's previous `event_count`/`last_synced_at` so existing events stay
/// visible) and returns `Ok(0)` — this is an expected, recoverable outcome,
/// not a job error. Likewise a parse failure records
/// [`SyncStatus::ParseError`] and returns `Ok(0)`. Only a genuine storage
/// failure (the database itself misbehaving) returns `Err`.
///
/// # Errors
///
/// Returns an error string if a storage call (upsert, prune, or recording the
/// sync state) fails.
pub async fn sync_one<F, Fut>(
    pool: &SqlitePool,
    now: OffsetDateTime,
    calendar: &StoredCalendar,
    fetch: &F,
) -> Result<usize, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, FetchError>>,
{
    // Delegate to the detailed helper and surface only the upserted count —
    // the number most useful to a caller driving a single calendar (e.g. a
    // manual "sync now" action), per this function's public contract.
    sync_one_detailed(pool, now, calendar, fetch)
        .await
        .map(|outcome| outcome.events_upserted)
}

/// The full orchestration behind [`sync_one`], also used by [`run_once`] so
/// it can accumulate the prune count into its [`SyncReport`] without a second
/// database round trip. See [`sync_one`]'s doc comment for the fetch/parse
/// failure contract — this function implements it directly (steps below
/// follow the task brief's numbered orchestration).
async fn sync_one_detailed<F, Fut>(
    pool: &SqlitePool,
    now: OffsetDateTime,
    calendar: &StoredCalendar,
    fetch: &F,
) -> Result<SyncOutcome, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, FetchError>>,
{
    let calendar_id = calendar.calendar.id;
    // Stringified once; reused for the source-column bind and the prune query.
    let calendar_id_str = calendar_id.to_string();

    // ── Step 1: fetch ───────────────────────────────────────────────────
    // On failure, the feed is unreachable this cycle. The household's
    // existing events for it must not disappear, so the previous
    // `event_count`/`last_synced_at` are carried forward unchanged — only
    // the status and error message change.
    let text = match fetch(calendar.calendar.url.clone()).await {
        // The happy path: hand the raw body text on to the parser below.
        Ok(text) => text,
        // The egress guards in `crate::feeds` (timeout, redirect limit, size
        // cap) all surface here as `FetchError`, alongside plain DNS/connect
        // failures and non-2xx statuses. Every one of them is "this cycle's
        // fetch did not work", not "the calendar is broken".
        Err(err) => {
            let state = CalendarSyncState {
                // Unchanged: the last GOOD sync time, not "now".
                last_synced_at: calendar.sync.last_synced_at,
                // The one status this branch ever records.
                last_status: SyncStatus::Unreachable,
                // `FetchError`'s Display gives a human-readable reason.
                last_error: Some(err.to_string()),
                // Unchanged: the household still sees its last-known count.
                event_count: calendar.sync.event_count,
            };
            update_calendar_sync_state(pool, calendar_id, &state)
                .await
                .map_err(|e| format!("update_calendar_sync_state: {e}"))?;
            // Nothing was ingested or pruned this cycle.
            return Ok(SyncOutcome {
                events_upserted: 0,
                events_pruned: 0,
            });
        }
    };

    // ── Step 2: parse ───────────────────────────────────────────────────
    // Same "keep what we had" contract as a fetch failure: a feed that
    // briefly serves garbage should not blank out the household's calendar.
    let parsed_events = match parse_feed(&text) {
        // The happy path: every VEVENT the parser could make sense of.
        Ok(events) => events,
        // Only `IcsError::NotCalendar` reaches here — a per-VEVENT problem
        // is already handled (and logged) inside `parse_feed` itself and
        // never surfaces as an `Err` at this level.
        Err(err) => {
            let state = CalendarSyncState {
                // Unchanged: the last GOOD sync time, not "now".
                last_synced_at: calendar.sync.last_synced_at,
                // The one status this branch ever records.
                last_status: SyncStatus::ParseError,
                // `IcsError`'s Display names exactly what was wrong.
                last_error: Some(err.to_string()),
                // Unchanged: the household still sees its last-known count.
                event_count: calendar.sync.event_count,
            };
            update_calendar_sync_state(pool, calendar_id, &state)
                .await
                .map_err(|e| format!("update_calendar_sync_state: {e}"))?;
            // Nothing was ingested or pruned this cycle.
            return Ok(SyncOutcome {
                events_upserted: 0,
                events_pruned: 0,
            });
        }
    };

    // ── Step 3: map ParsedEvent → Event ─────────────────────────────────
    let events = build_events(&parsed_events, &calendar_id_str, now);

    // ── Step 4: upsert ───────────────────────────────────────────────────
    upsert_external_events(pool, &events)
        .await
        .map_err(|e| format!("upsert_external_events: {e}"))?;

    // ── Step 5: prune ────────────────────────────────────────────────────
    // `uids` is every UID the feed currently reports, independent of
    // whether that event happened to fail the `EventBuilder` mapping above —
    // a UID the feed still lists must never be pruned just because Amity
    // could not build a row for it this cycle.
    let uids: Vec<String> = parsed_events.iter().map(|p| p.uid.clone()).collect();
    let events_pruned = prune_events_missing_from_feed(pool, &calendar_id_str, &uids)
        .await
        .map_err(|e| format!("prune_events_missing_from_feed: {e}"))?;

    // ── Step 6: expand + materialise instances ──────────────────────────
    materialise_instances(pool, &parsed_events, &calendar_id_str, now).await?;

    // ── Step 7: record success ──────────────────────────────────────────
    // A household calendar never approaches u32::MAX events; the fallback
    // exists only so this line cannot panic, not because it is expected to
    // ever trigger.
    let event_count = u32::try_from(events.len()).unwrap_or(u32::MAX);
    let state = CalendarSyncState {
        // This cycle's fetch-and-parse succeeded right now.
        last_synced_at: Some(now),
        // The one status this branch ever records.
        last_status: SyncStatus::Ok,
        // A clean sync clears any error recorded by a prior failed attempt.
        last_error: None,
        // How many events this cycle's feed produced.
        event_count,
    };
    update_calendar_sync_state(pool, calendar_id, &state)
        .await
        .map_err(|e| format!("update_calendar_sync_state: {e}"))?;

    // Report both counts back to the caller (see `SyncOutcome`'s doc comment
    // for why `sync_one` itself only surfaces the first of the two).
    Ok(SyncOutcome {
        events_upserted: events.len(),
        events_pruned,
    })
}

/// Map a feed's `ParsedEvent`s into `Event`s ready for `upsert_external_events`
/// (step 3 of [`sync_one_detailed`]'s orchestration).
///
/// `Event::recurrence` stays `None` here even for a recurring `ParsedEvent` —
/// per `amity_core::event`'s doc comment, external recurrence is trusted
/// entirely to the source and never copied onto the Event's own recurrence
/// field. The RRULE is still used later, just via `expand_external` on the
/// `ParsedEvent` directly (see [`materialise_instances`]) rather than through
/// `Event.recurrence`.
///
/// A single malformed mapping (e.g. an empty title after trimming) is logged
/// and skipped rather than aborting the whole calendar, matching
/// `parse_feed`'s own "one bad event never sinks a feed" policy.
fn build_events(
    parsed_events: &[ParsedEvent],
    calendar_id: &str,
    now: OffsetDateTime,
) -> Vec<Event> {
    parsed_events
        .iter()
        .filter_map(|parsed| {
            // Required/optional fields carried straight from the parsed
            // event; the source ties this row back to its feed + UID.
            let mut builder = EventBuilder::new()
                .title(parsed.summary.clone())
                .start_at(parsed.start)
                .all_day(parsed.all_day)
                .now(now)
                .source(EventSource::ics(parsed.uid.clone(), calendar_id, now));
            if let Some(end) = parsed.end {
                builder = builder.end_at(end);
            }
            match builder.build() {
                Ok(event) => Some(event),
                Err(e) => {
                    tracing::warn!(
                        calendar_id,
                        uid = %parsed.uid,
                        error = %e,
                        "skipping event that failed to build from ICS feed"
                    );
                    None
                }
            }
        })
        .collect()
}

/// Expand every parsed event to its instants within the horizon and persist
/// them as `EventInstance` rows (step 6 of [`sync_one_detailed`]'s
/// orchestration).
///
/// `upsert_external_events` preserves an existing row's id on conflict (only
/// mutable columns are overwritten — see that function's doc comment), so a
/// freshly-built `Event` does NOT reliably carry the id its row actually has
/// in the database for an already-known UID. Reading the calendar's events
/// back by `(calendar_id, external_id)` gives the authoritative id each
/// instance must reference.
///
/// Before re-materialising an event's instances, its not-yet-happened
/// (`scheduled_at >= now`) instance rows are swept via
/// `delete_future_event_instances`. Without this, a re-sync only ever adds
/// rows (`upsert_event_instances` is `INSERT OR IGNORE`, keyed on
/// `(event_id, scheduled_at)`), so a feed edit — DTSTART/RRULE change or a
/// newly-added EXDATE — would leave the old, now-stale instances in place
/// forever alongside the fresh ones (Important #2, final whole-branch
/// review). Past instances are untouched: the delete's lower bound is `now`,
/// matching `expand_external`'s own `from` bound below, so history is never
/// disturbed by a routine re-sync.
///
/// # Errors
///
/// Returns an error string if reading the events back, sweeping stale
/// instances, or upserting the fresh ones fails.
async fn materialise_instances(
    pool: &SqlitePool,
    parsed_events: &[ParsedEvent],
    calendar_id: &str,
    now: OffsetDateTime,
) -> Result<(), String> {
    // Re-read every event; at household scale this is a small table, and
    // reusing the same "load everything, filter in memory" approach as
    // `list_events`'s own callers keeps this module free of a bespoke
    // by-calendar query.
    let stored_events = list_events(pool)
        .await
        .map_err(|e| format!("list_events: {e}"))?;
    // Keep only this calendar's rows, keyed by their feed UID — the lookup
    // `expand_external`'s instances need below.
    let event_id_by_uid: std::collections::HashMap<&str, EventId> = stored_events
        .iter()
        .filter(|e| e.source.calendar_id.as_deref() == Some(calendar_id))
        .filter_map(|e| e.source.external_id.as_deref().map(|uid| (uid, e.id)))
        .collect();

    // The same forward window used for native events/tasks (see
    // `HORIZON_DAYS`'s doc comment).
    let horizon = now + Duration::days(HORIZON_DAYS);
    // One instance row per (event, occurrence) pair across the whole feed.
    let mut instances = Vec::new();
    for parsed in parsed_events {
        // An event whose id we cannot resolve was dropped by `build_events`
        // (a malformed mapping) — it has no row to attach instances to.
        let Some(&event_id) = event_id_by_uid.get(parsed.uid.as_str()) else {
            continue;
        };
        // Sweep this event's not-yet-happened instances BEFORE expanding the
        // fresh set below, using the authoritative `event_id` resolved above
        // (the same id the upsert below will reference) — see this
        // function's doc comment for why a re-sync needs this. A per-event
        // delete is fine at household scale (a handful of calendars, each
        // with a small event count).
        delete_future_event_instances(pool, event_id, now)
            .await
            .map_err(|e| format!("delete_future_event_instances: {e}"))?;
        // A one-shot event yields its single start (if in-window); a
        // recurring one yields every occurrence the rrule crate produces
        // between `now` and `horizon` — see `expand_external`'s doc comment.
        for scheduled_at in expand_external(parsed, now, horizon) {
            instances.push(EventInstance {
                // Fresh, time-ordered id per instance row.
                id: uuid::Uuid::now_v7().to_string(),
                // The authoritative id resolved above, not `parsed`'s own.
                event_id,
                // The concrete occurrence instant from expand_external.
                scheduled_at,
            });
        }
    }
    // One idempotent bulk upsert for the whole calendar's instances.
    upsert_event_instances(pool, &instances)
        .await
        .map_err(|e| format!("upsert_event_instances: {e}"))
}

// ─── One full pass ──────────────────────────────────────────────────────────

/// Run one sync pass against every *enabled* calendar, anchored at `now`.
///
/// Loads the calendar list, skips disabled ones (the sync job never touches
/// them — see `amity_core::calendar::CalendarBuilder::enabled`), and calls
/// [`sync_one_detailed`] for each, accumulating a [`SyncReport`]. A
/// per-calendar storage failure is logged and the loop continues — one
/// misbehaving calendar's database error must not stop the rest of the
/// household's feeds from syncing. (A feed being unreachable or unparsable is
/// NOT such a failure — see `sync_one`'s doc comment; that path always
/// returns `Ok` and simply contributes zero to the counts.)
///
/// # Errors
///
/// Returns an error string only if the calendar list itself cannot be read —
/// an unexpected storage-level failure with nothing to loop over.
pub async fn run_once<F, Fut>(
    pool: &SqlitePool,
    now: OffsetDateTime,
    fetch: F,
) -> Result<SyncReport, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, FetchError>>,
{
    // Every subscribed calendar, enabled or not — filtered below.
    let calendars = list_calendars(pool)
        .await
        .map_err(|e| format!("list_calendars: {e}"))?;

    let mut report = SyncReport::default();

    for calendar in calendars.iter().filter(|c| c.calendar.enabled) {
        match sync_one_detailed(pool, now, calendar, &fetch).await {
            Ok(outcome) => {
                report.calendars_synced += 1;
                report.events_upserted += outcome.events_upserted;
                report.events_pruned += outcome.events_pruned;
            }
            // A storage-level failure for this one calendar; log and move on
            // so the rest of the household's feeds still get synced.
            Err(e) => {
                tracing::warn!(
                    calendar_id = %calendar.calendar.id,
                    error = %e,
                    "calendar sync failed"
                );
            }
        }
    }

    Ok(report)
}

// ─── Spawned scheduler ──────────────────────────────────────────────────────

/// Spawn the calendar sync job: run once on startup, then every 6 hours.
///
/// Identical shape to `jobs::recurrence_horizon::spawn` — a `tokio` interval
/// whose first tick fires immediately, so freshly subscribed feeds are synced
/// as soon as the service starts. Failures are logged, never fatal; the next
/// tick retries. Passes the real [`feeds::fetch`] as the injected fetch
/// function (tests inject a fixture closure instead — see
/// `tests/calendar_sync.rs`).
pub fn spawn(pool: SqlitePool) {
    // Detach a background task; it lives as long as the runtime does.
    tokio::spawn(async move {
        // A 6-hour interval; `interval` fires its first tick right away.
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(RUN_INTERVAL_SECS));
        loop {
            // Wait for the next tick (immediate on the first iteration).
            ticker.tick().await;
            // Anchor this pass at the current wall-clock time.
            let now = OffsetDateTime::now_utc();
            // Run the pass; log the outcome either way.
            match run_once(&pool, now, feeds::fetch).await {
                // Success: record what the pass did at info level.
                Ok(report) => tracing::info!(
                    calendars = report.calendars_synced,
                    upserted = report.events_upserted,
                    pruned = report.events_pruned,
                    "calendar sync ran"
                ),
                // Failure: log and let the next tick retry.
                Err(e) => tracing::error!(error = %e, "calendar sync failed"),
            }
        }
    });
}
