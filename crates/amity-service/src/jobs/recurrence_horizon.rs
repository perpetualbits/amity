// jobs/recurrence_horizon.rs — the recurrence horizon maintenance job.
//
// Recurring tasks materialise their instances at creation time, out to a 60-day
// horizon (see api/task.rs and ADR-0002 §materialisation-strategy). Without a
// periodic job that horizon never moves forward: after 60 days a daily task
// would silently run out of instances and drop off the Today view. This job is
// the piece ADR-0002 named but Task 2 left as a TODO.
//
// On each run it, for every recurring task:
//   • re-materialises instances from `now` out to `now + HORIZON_DAYS`
//     (idempotent — INSERT OR IGNORE skips instances that already exist); and
// then, once across all tasks:
//   • prunes instances older than `now - RETENTION_DAYS`, keeping a short
//     backward window so recent CompletionLog references still resolve.
//
// `run_once` holds all the testable logic and takes an explicit `now` (clock
// injection, matching the rest of the codebase). `spawn` is the thin wrapper
// that runs it on startup and then daily.

// The shared connection pool.
use sqlx::SqlitePool;
// `Duration`/`OffsetDateTime` drive the horizon and prune-cutoff arithmetic.
use time::{Duration, OffsetDateTime};

// `Task` is what we materialise from; `TaskFilter`/`list_tasks` load the set.
use amity_core::recurrence_materialiser::materialise_instances;
use amity_core::task::Task;
use amity_storage::task::{TaskFilter, list_tasks};
// Instance persistence: build rows, upsert them, and prune the aged-out ones.
use amity_storage::task_instance::{TaskInstance, prune_old_instances, upsert_task_instances};

// ─── Tuning constants ───────────────────────────────────────────────────────

/// How far forward instances are kept materialised, in days.
///
/// Matches the 60-day horizon used at task-creation time so a task created today
/// and a task rolled forward by this job present the same two-month window.
const HORIZON_DAYS: i64 = 60;

/// How far back completed/old instances are retained before pruning, in days.
///
/// A short backward window keeps recent instances around so `CompletionLog`
/// references still resolve; anything older is derived data and can be rebuilt.
const RETENTION_DAYS: i64 = 30;

/// How often the spawned job runs, in seconds (24 hours).
const RUN_INTERVAL_SECS: u64 = 24 * 60 * 60;

// ─── Report ─────────────────────────────────────────────────────────────────

/// What a single maintenance pass did — for logging and for tests to assert on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HorizonReport {
    /// Total tasks examined (recurring and one-shot alike).
    pub tasks_seen: usize,
    /// Instances handed to the idempotent upsert across all recurring tasks.
    ///
    /// This counts materialised occurrences, not newly-inserted rows — the
    /// `INSERT OR IGNORE` may skip ones that already exist from a prior run.
    pub instances_materialised: usize,
    /// Instances deleted by the single prune query at the end of the pass.
    pub instances_pruned: u64,
}

// ─── One maintenance pass ───────────────────────────────────────────────────

/// Run one horizon-maintenance pass against `pool`, anchored at `now`.
///
/// Materialises every recurring task forward to the horizon, then prunes
/// aged-out instances. Returns a [`HorizonReport`]. A per-task materialisation
/// failure is logged and skipped so one bad task cannot stall the whole pass;
/// a prune failure aborts the pass with an error.
///
/// # Errors
///
/// Returns an error string if the task list cannot be read or the prune query
/// fails — both are unexpected storage-level failures.
pub async fn run_once(pool: &SqlitePool, now: OffsetDateTime) -> Result<HorizonReport, String> {
    // The forward horizon and the backward retention cutoff, both relative to now.
    let horizon = now + Duration::days(HORIZON_DAYS);
    let prune_before = now - Duration::days(RETENTION_DAYS);

    // Load every task. `materialise_task_to_horizon` no-ops on one-shot tasks,
    // so there is no need to pre-filter to recurring ones here.
    let tasks = list_tasks(pool, &TaskFilter::default())
        .await
        .map_err(|e| format!("list_tasks: {e}"))?;

    // Accumulate the outcome as we go.
    let mut report = HorizonReport::default();

    for task in &tasks {
        // Count every task examined, recurring or not.
        report.tasks_seen += 1;
        // Roll this task's instances forward to the horizon (idempotent).
        match materialise_task_to_horizon(pool, task, now, horizon).await {
            // Add the materialised count to the running total.
            Ok(count) => report.instances_materialised += count,
            // A single bad task must not stall the pass — log and continue.
            Err(e) => {
                tracing::warn!(task_id = %task.id, error = %e, "horizon materialisation failed");
            }
        }
    }

    // Prune aged-out instances across all tasks in one query at the end.
    // A prune failure is an unexpected storage error, so it aborts the pass.
    report.instances_pruned = prune_old_instances(pool, prune_before)
        .await
        .map_err(|e| format!("prune_old_instances: {e}"))?;

    Ok(report)
}

// ─── Shared materialisation helper ──────────────────────────────────────────

/// Materialise a task's instances from `from` out to `horizon` and upsert them.
///
/// Returns the number of instances handed to the upsert (0 for a one-shot task,
/// which has no recurrence rule). Shared by the task-creation path and this job
/// so the "datetimes → `TaskInstance` → upsert" mapping lives in exactly one
/// place. The holiday-skip set is empty until the NL holiday source is wired in
/// (ADR-0002 §future-work).
///
/// # Errors
///
/// Returns an error string if the core materialiser rejects the rule or the
/// upsert fails.
pub(crate) async fn materialise_task_to_horizon(
    pool: &SqlitePool,
    task: &Task,
    from: OffsetDateTime,
    horizon: OffsetDateTime,
) -> Result<usize, String> {
    // A one-shot task has no rule and therefore no instances to materialise.
    let Some(rule) = task.recurrence.as_ref() else {
        return Ok(0);
    };

    // No holiday skip set yet — the stub is an empty set, so all dates are kept.
    let skip_dates = std::collections::HashSet::new();

    // Core materialiser: RRULE + DTSTART + IANA timezone → scheduled datetimes,
    // preserving wall-clock time across DST transitions.
    let datetimes = materialise_instances(rule, from, horizon, &skip_dates)
        .map_err(|e| format!("materialise_instances: {e}"))?;

    // Wrap each datetime into a storable instance row.
    let instances: Vec<TaskInstance> = datetimes
        .into_iter()
        .map(|scheduled_at| TaskInstance {
            // Fresh, time-ordered id per instance row.
            id: uuid::Uuid::now_v7().to_string(),
            // Link back to the parent recurring task.
            task_id: task.id,
            // The scheduled occurrence time.
            scheduled_at,
            // Inherit the parent's default assignee; overridable per-instance.
            current_assignee_id: task.current_assignee_id,
        })
        .collect();

    // Remember how many we attempted before the vec is moved into the upsert.
    let count = instances.len();
    // Persist idempotently; the (task_id, scheduled_at) UNIQUE constraint dedupes.
    upsert_task_instances(pool, &instances)
        .await
        .map_err(|e| format!("upsert_task_instances: {e}"))?;
    Ok(count)
}

// ─── Spawned scheduler ──────────────────────────────────────────────────────

/// Spawn the maintenance job: run once on startup, then every 24 hours.
///
/// Uses a `tokio` interval whose first tick fires immediately, so the horizon is
/// extended as soon as the service starts. Failures are logged, never fatal —
/// the next tick retries. The task owns its own clone of the pool and runs for
/// the lifetime of the process.
pub fn spawn(pool: SqlitePool) {
    // Detach a background task; it lives as long as the runtime does.
    tokio::spawn(async move {
        // A 24-hour interval; `interval` fires its first tick right away.
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(RUN_INTERVAL_SECS));
        loop {
            // Wait for the next tick (immediate on the first iteration).
            ticker.tick().await;
            // Anchor this pass at the current wall-clock time.
            let now = OffsetDateTime::now_utc();
            // Run the pass; log the outcome either way.
            match run_once(&pool, now).await {
                // Success: record what the pass did at info level.
                Ok(report) => tracing::info!(
                    tasks = report.tasks_seen,
                    materialised = report.instances_materialised,
                    pruned = report.instances_pruned,
                    "recurrence horizon maintenance ran"
                ),
                // Failure: log and let the next tick retry.
                Err(e) => tracing::error!(error = %e, "recurrence horizon maintenance failed"),
            }
        }
    });
}
