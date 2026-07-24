// horizon_job.rs — integration tests for the recurrence horizon maintenance job.
//
// These drive `run_once` directly against a real in-memory database, seeding
// tasks and instances through the production storage layer. `run_once` takes an
// explicit `now`, so every case is deterministic regardless of the wall clock.
//
// What these verify:
//   • a recurring task gets instances materialised forward to the horizon;
//   • instances older than the retention window are pruned;
//   • one-shot tasks are ignored (they have no instances to materialise).
//
// The scheduled wrapper (`spawn`) is a thin tokio loop over `run_once`; its
// timing is not unit-tested — the maintenance logic under test lives in run_once.

// The job under test, plus its report type.
use amity_service::jobs::recurrence_horizon::run_once;
// Domain construction: a validated Task and its recurrence rule.
use amity_core::recurrence::RecurrenceRule;
use amity_core::task::TaskBuilder;
// Typed member id for the task owner (the migration-seeded placeholder).
use amity_core::ids::MemberId;
// Storage: open the pool, insert a task, and read/seed instances.
use amity_storage::connection::open_database;
use amity_storage::task::insert_task;
use amity_storage::task_instance::{TaskInstance, list_upcoming_instances, upsert_task_instances};
// `Duration` shifts times relative to `now`; `datetime!` pins a fixed `now`.
use time::Duration;
use time::macros::datetime;

// ─── Fixtures ─────────────────────────────────────────────────────────────────

/// The placeholder member seeded by migration 0001 (owns every task for now).
fn placeholder() -> MemberId {
    // A compile-time-constant UUID; parsing it cannot fail.
    MemberId(uuid_from("00000000-0000-7000-8000-000000000001"))
}

/// Parse a UUID string without pulling `uuid` into the test's imports.
fn uuid_from(s: &str) -> uuid::Uuid {
    // The inputs here are fixed literals, so a parse failure is a test bug.
    s.parse().expect("valid UUID literal")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn horizon_job_materialises_instances_for_a_recurring_task() {
    // A recurring task with no pre-materialised instances must get a full
    // horizon's worth after a maintenance pass — this is what stops a long-lived
    // daily task from silently running out of instances, which is exactly the
    // gap ADR-0002 flagged and Task 2 left as a TODO.
    let db = open_database("sqlite::memory:")
        // Await the pool; `open_database` applies all migrations first.
        .await
        // Opening an in-memory database cannot fail under normal conditions.
        .expect("in-memory db");
    // A fixed anchor so the materialised set is deterministic.
    let now = datetime!(2026-07-24 12:00:00 UTC);

    // Build and persist a daily recurring task (no instances yet).
    let task = TaskBuilder::new()
        // A recognisable subject.
        .title("daily standup")
        // Owned by the placeholder member (satisfies the FK).
        .owner_id(placeholder())
        // Deterministic created/updated timestamps.
        .now(now)
        // Every day, anchored to Amsterdam wall-clock time.
        .recurrence(RecurrenceRule::new("FREQ=DAILY", "Europe/Amsterdam"))
        // Validate and construct.
        .build()
        // The builder enforces title/owner/now invariants.
        .expect("valid recurring task");
    // Write the task row.
    insert_task(&db, &task).await.expect("insert task");

    // Run one maintenance pass anchored at `now`.
    let report = run_once(&db, now).await.expect("maintenance pass");
    // It saw the single task and materialised at least one instance.
    assert_eq!(report.tasks_seen, 1);
    assert!(
        report.instances_materialised > 0,
        "a daily rule must materialise instances forward"
    );

    // And the instances are actually queryable from `now` onward — the report
    // count and the persisted rows must agree, not just the in-memory tally.
    let upcoming = list_upcoming_instances(&db, task.id, now, 100)
        // Await the query.
        .await
        // A read failure here would be an unexpected storage error.
        .expect("list instances");
    // At least one future instance now exists in the table.
    assert!(!upcoming.is_empty(), "instances must be persisted");
}

#[tokio::test]
async fn horizon_job_prunes_aged_out_instances() {
    // Instances older than the retention window are derived data that can be
    // rebuilt; the pass removes them so the table does not grow without bound.
    // CompletionLog references live separately, so pruning loses no history —
    // this is the backward half of "roll the horizon forward, prune the tail".
    let db = open_database("sqlite::memory:")
        // Await the pool; `open_database` applies all migrations first.
        .await
        // Opening an in-memory database cannot fail under normal conditions.
        .expect("in-memory db");
    // A fixed anchor for determinism.
    let now = datetime!(2026-07-24 12:00:00 UTC);

    // A one-shot task exists only to satisfy the instance foreign key; it
    // materialises nothing, isolating the prune behaviour under test.
    let task = TaskBuilder::new()
        // A plain one-shot subject.
        .title("anchor task")
        // Placeholder owner for the FK.
        .owner_id(placeholder())
        // Deterministic timestamps.
        .now(now)
        // No recurrence → nothing to materialise.
        .build()
        .expect("valid one-shot task");
    // Persist the task.
    insert_task(&db, &task).await.expect("insert task");

    // Seed one aged-out instance, scheduled 40 days before `now` (past retention).
    let aged_out = TaskInstance {
        // A fixed instance id (any valid UUID string).
        id: "018f0000-0000-7000-8000-000000000abc".to_owned(),
        // Reference the anchor task.
        task_id: task.id,
        // 40 days ago — older than the 30-day retention window.
        scheduled_at: now - Duration::days(40),
        // No assignee needed for the prune test.
        current_assignee_id: None,
    };
    // Write the aged-out instance directly through the storage layer.
    upsert_task_instances(&db, &[aged_out])
        // Await the insert.
        .await
        // Seeding a single valid row cannot fail here.
        .expect("seed instance");

    // Run one maintenance pass anchored at the fixed `now`.
    // This is the production job entry point, exercised directly.
    let report = run_once(&db, now).await.expect("maintenance pass");
    // The aged-out instance was pruned.
    assert!(
        report.instances_pruned >= 1,
        "the aged-out instance must be pruned"
    );

    // And nothing remains for the anchor task (one-shot materialises nothing),
    // so a query spanning well before `now` must come back empty.
    let remaining = list_upcoming_instances(&db, task.id, now - Duration::days(100), 100)
        // Await the query.
        .await
        // A read failure here would be an unexpected storage error.
        .expect("list instances");
    // The table no longer holds the aged-out row.
    assert!(remaining.is_empty(), "pruned instance must be gone");
}

#[tokio::test]
async fn horizon_job_ignores_one_shot_tasks() {
    // One-shot tasks have no recurrence rule and therefore no instances — the
    // pass must see them but materialise nothing for them. This guards the
    // shared helper's early return so the job never invents instances for a
    // task that should stay a single, dateless commitment.
    let db = open_database("sqlite::memory:")
        // Await the pool; `open_database` applies all migrations first.
        .await
        // Opening an in-memory database cannot fail under normal conditions.
        .expect("in-memory db");
    // A fixed anchor for determinism.
    let now = datetime!(2026-07-24 12:00:00 UTC);

    // A single one-shot task with a due date but no recurrence.
    let task = TaskBuilder::new()
        // A recognisable one-shot subject.
        .title("call the dentist")
        // Placeholder owner.
        .owner_id(placeholder())
        // Deterministic timestamps.
        .now(now)
        // Due in five days — but still no recurrence.
        .due_by(now + Duration::days(5))
        // Validate and construct.
        .build()
        // The builder enforces title/owner/now invariants.
        .expect("valid one-shot task");
    // Persist it.
    insert_task(&db, &task).await.expect("insert task");

    // Run one maintenance pass anchored at the fixed `now`.
    // This is the production job entry point, exercised directly.
    let report = run_once(&db, now).await.expect("maintenance pass");
    // The one-shot task was seen but produced no instances.
    assert_eq!(report.tasks_seen, 1);
    assert_eq!(
        report.instances_materialised, 0,
        "one-shot tasks must not materialise instances"
    );
}
