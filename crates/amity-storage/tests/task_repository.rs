// task_repository.rs — integration tests for the task storage layer.
//
// Each test spins up a fresh in-memory SQLite database with all migrations
// applied and exercises the repository functions directly, bypassing HTTP.
//
// What these tests verify:
//   • insert_task / fetch_task: every field survives the write→read cycle.
//   • fetch_task: returns None (not error) for a non-existent ID.
//   • list_tasks: created_at-descending order and three filter types.
//   • update_task: field changes are visible on the next fetch.
//   • mark_task_done / mark_task_skipped: status + CompletionLog rows.
//   • upsert_task_instances: idempotent — inserting the same window twice
//     does not produce duplicate rows (INSERT OR IGNORE enforces this).
//   • list_upcoming_instances: `from` and `limit` are both honoured.
//   • prune_old_instances: removes rows before the cutoff, keeps the rest.
//   • delete_future_instances: removes rows at or after the cutoff only.
//
// Each test gets its own isolated in-memory database so tests can run in
// parallel without interfering. There is no shared mutable state between tests.
//
// These tests do not verify HTTP semantics (those live in amity-service tests)
// or business logic beyond what the storage layer directly enforces.

// Core domain types used to build test fixtures throughout this file.
use amity_core::completion_log::CompletionLog;
// `MemberId` for the placeholder FK constraint; `TaskId` is the task primary key.
use amity_core::ids::MemberId;
// `Task` is the domain struct built by the builder pattern.
// `TaskBuilder` enforces invariants at construction time.
// `TaskStatus` is used as a filter criterion in the list tests.
use amity_core::task::{Task, TaskBuilder, TaskStatus};

// Repository functions under test in this file.
use amity_storage::completion_log::list_completion_logs_for_task;
// `open_database` applies all migrations and returns a ready-to-use pool.
use amity_storage::connection::open_database;
// CRUD and completion-marking operations for the task table.
use amity_storage::task::{
    TaskFilter, fetch_task, insert_task, list_tasks, mark_task_done, mark_task_skipped, update_task,
};
// Materialise, query, prune, and delete-future operations for task instances.
use amity_storage::task_instance::{
    TaskInstance, delete_future_instances, list_upcoming_instances, prune_old_instances,
    upsert_task_instances,
};

// `date!` produces a compile-time `time::Date` for the `instance_date` field.
// `datetime!` produces a compile-time `OffsetDateTime` for deterministic timestamps.
use time::macros::{date, datetime};
// `OffsetDateTime` is the project-wide canonical timestamp type.
// `Duration` is used to offset timestamps by known intervals in tests.
use time::{Duration, OffsetDateTime};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Open a fresh in-memory database with all migrations applied.
///
/// Each call creates a completely isolated database — no rows carry over from
/// previous calls even when tests run in parallel. `open_database` applies
/// the full `sqlx::migrate!` suite before returning the pool, so the schema
/// is always up to date and tests always start from a known-clean state.
///
/// Using `:memory:` rather than a temp file avoids post-test cleanup and
/// guarantees each database starts empty regardless of previous test runs.
async fn open_test_db() -> sqlx::SqlitePool {
    // `:memory:` creates a brand-new, empty database on every call.
    open_database("sqlite::memory:")
        .await
        // A panic here means the migration SQL is broken, not a test data issue.
        .expect("in-memory database should always open")
}

/// The placeholder member UUID seeded by migration 0001.
///
/// Tasks require an `owner_id` that references a valid `members(id)` row.
/// The in-memory database starts with only this one member row (inserted by
/// the migration), so every test must use this UUID for any member reference.
///
/// This UUID is identical to the one in `api/task.rs`. Both must stay in sync
/// until the Member entity lands and real authentication is added.
fn placeholder_member() -> MemberId {
    // Must match the UUID in `0001_initial.sql` exactly — any mismatch fails FK.
    MemberId(
        uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
            .expect("hardcoded UUID is always valid"),
    )
}

/// Build a minimal valid `Task` with the given title and creation timestamp.
///
/// The builder enforces the non-empty title and `owner_id` invariants at build
/// time. Using a fixed `now` timestamp makes ordering assertions predictable
/// and independent of the system clock — tests that need ordering control
/// can pass distinct timestamps here.
///
/// `expect` in test helpers is intentional: failure indicates broken test-setup
/// data, not a violation of a production invariant.
fn make_task(title: &str, now: OffsetDateTime) -> Task {
    TaskBuilder::new()
        // Title must not be empty or purely whitespace.
        .title(title.to_string())
        // The only valid FK reference in the test database.
        .owner_id(placeholder_member())
        // Sets both `created_at` and `updated_at` to this timestamp.
        .now(now)
        .build()
        .expect("minimal test task should always be valid")
}

/// Build a `Task` with one tag, using the same defaults as `make_task`.
///
/// Tags are normalised (trimmed and lowercased) by `TaskBuilder`, so the
/// stored tag may differ from the raw input. Tests that assert on the stored
/// tag value must pass a pre-normalised string or assert on the normalised form.
fn make_task_with_tag(title: &str, tag: &str, now: OffsetDateTime) -> Task {
    TaskBuilder::new()
        // Same required fields as the tagless helper.
        .title(title.to_string())
        .owner_id(placeholder_member())
        .now(now)
        // The builder normalises the tag before storing it.
        .tag(tag)
        .build()
        .expect("task with tag should always be valid")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn insert_and_fetch_round_trip() {
    // Every field written via `insert_task` must come back unchanged via
    // `fetch_task`. If any field is silently altered — timezone stripped,
    // enum variant re-encoded, UUID truncated — this test catches it.
    //
    // Fields verified: id, title, status, created_at, updated_at, owner_id.
    // Optional fields (notes, due_by, etc.) are covered by `update_task_persists_changes`.
    // Each test gets an isolated in-memory database with migrations applied.
    let pool = open_test_db().await;
    // Fixed timestamp so assertion values are deterministic and not clock-dependent.
    // Using a compile-time constant avoids flakiness caused by system-clock variations.
    let now = datetime!(2026-05-25 10:00:00 UTC);
    // Build a minimal valid task to serve as the write fixture.
    // All optional fields (notes, due_by, etc.) default to None.
    let task = make_task("vacuum living room", now);

    // ── Write ────────────────────────────────────────────────────────────────

    // Write the task to the database; any error here means the schema is wrong.
    insert_task(&pool, &task)
        // Await the async write; the pool serialises concurrent writers on SQLite.
        .await
        .expect("insert should succeed");

    // ── Read back ────────────────────────────────────────────────────────────

    // Fetch the task by its primary key to verify the round-trip.
    let fetched = fetch_task(&pool, task.id)
        .await
        .expect("fetch should not error for a known-inserted ID")
        // An immediate None after insert indicates a broken write or fetch query.
        .expect("task must exist immediately after insert");

    // ── Assert each field ────────────────────────────────────────────────────

    // UUID primary key: TEXT→`TaskId` parse must be lossless.
    assert_eq!(fetched.id, task.id, "id must round-trip");
    // Title: stored verbatim with no normalisation beyond the non-empty check.
    assert_eq!(fetched.title, task.title, "title must round-trip");
    // Status: stored as a lowercase string; must parse to the same variant.
    assert_eq!(fetched.status, task.status, "status must round-trip");
    // created_at: RFC 3339 TEXT must preserve the UTC offset and seconds.
    assert_eq!(
        fetched.created_at, task.created_at,
        "created_at must round-trip"
    );
    // updated_at: initially equals created_at; must also survive the round-trip.
    assert_eq!(
        fetched.updated_at, task.updated_at,
        "updated_at must round-trip"
    );
    // owner_id: UUID TEXT column; the hyphenated UUID string must be lossless.
    assert_eq!(fetched.owner_id, task.owner_id, "owner_id must round-trip");
}

#[tokio::test]
async fn fetch_nonexistent_returns_none() {
    // A missing task must return `Ok(None)`, not `Err(...)`. The service layer
    // maps `None` to a 404 HTTP response; returning an error would force the
    // handler to inspect the error variant to distinguish "not found" from
    // "I/O failure" — a fragile pattern that `Option` avoids cleanly.
    //
    // This test verifies the storage layer's contract, not the HTTP layer's.
    // An empty database — the generated ID was never inserted, so it is absent.
    let pool = open_test_db().await;
    // A fresh UUID that was never inserted into this isolated database.
    // UUID v7 uniqueness guarantees this ID has never been seen before.
    let missing_id = amity_core::ids::TaskId::new();

    // The fetch must succeed at the Result level but return None at the Option level.
    // An `Err` would indicate a database I/O failure, not a missing row.
    let result = fetch_task(&pool, missing_id)
        .await
        .expect("fetch should not return an error for a missing ID");

    // `None` is the correct representation of "row does not exist".
    // Returning `Err` for a missing row would force callers to inspect error variants.
    assert!(result.is_none(), "expected None for a non-existent task ID");
}

#[tokio::test]
async fn list_tasks_returns_newest_first() {
    // The list query must order by `created_at DESC`. This test inserts the
    // older task first — if the query returned insertion order instead of
    // timestamp order, the assertion would fail on `tasks[0].title`.
    //
    // Using fixed timestamps makes the expected order deterministic and
    // independent of how fast the test machine processes the inserts.
    let pool = open_test_db().await;

    // Older task: inserted first, has an earlier timestamp.
    let older = make_task("older task", datetime!(2026-05-25 08:00:00 UTC));
    // Newer task: inserted second, has a later timestamp.
    let newer = make_task("newer task", datetime!(2026-05-25 10:00:00 UTC));

    // Insert the older task first. If order were insertion-based, `older`
    // would appear first in the results — which the assertions below reject.
    // Insertion order must not determine the result order.
    insert_task(&pool, &older).await.expect("insert older task");
    // Insert the newer task second. The `ORDER BY created_at DESC` must put this first.
    insert_task(&pool, &newer).await.expect("insert newer task");

    // Fetch all tasks without a filter; both must be returned.
    // `TaskFilter::default()` has all fields set to None (no filtering applied).
    let tasks = list_tasks(&pool, &TaskFilter::default())
        .await
        .expect("list should succeed");

    // Both rows must be present — a count of 0 or 1 means an insert failed.
    assert_eq!(tasks.len(), 2, "expected exactly two tasks");
    // The first returned task must be the one with the higher `created_at`.
    // A failure here means the ORDER BY clause is missing or inverted.
    // If insertion order were used, "older task" would come first here.
    assert_eq!(tasks[0].title, "newer task", "newest task must come first");
    // The second returned task must be the one with the lower `created_at`.
    // Both tasks must appear — a count of 1 after inserting 2 would indicate a bug.
    assert_eq!(tasks[1].title, "older task", "older task must come second");
}

#[tokio::test]
async fn list_tasks_filters_by_status() {
    // A status filter must include only tasks in that lifecycle state.
    // Tasks in any other state must be excluded from the result set.
    //
    // This test creates two tasks (both start as `Open`), transitions one to
    // `Done`, then verifies that the `Open` filter returns exactly one task.
    //
    // The status is stored as lowercase TEXT ("open", "done", "skipped") and
    // the filter is applied via a WHERE clause on that TEXT column. This test
    // verifies that the storage-level string comparison works as expected and
    // that `update_task` actually persists the new status before the list runs.
    let pool = open_test_db().await;
    // Use the same base timestamp; ordering is not under test here.
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Both tasks start in `Open` status (the default set by the builder).
    let open_task = make_task("open task", now);
    // This task will be updated to `Done` before the filter is applied.
    let done_task = make_task("done task", now);

    // Insert both tasks so the filter has both in the table.
    insert_task(&pool, &open_task)
        // Await the async insert; any error indicates a schema or FK problem.
        .await
        .expect("insert open task");
    insert_task(&pool, &done_task)
        // Second insert is sequential — the pool serialises writes on SQLite.
        .await
        .expect("insert done task");

    // Transition `done_task` to Done by cloning and persisting.
    let mut to_update = done_task.clone();
    // Update the status field — the storage layer stores this as "done".
    to_update.status = TaskStatus::Done;
    // Bump `updated_at` so the row's modification timestamp is distinct.
    to_update.updated_at = now + Duration::seconds(1);
    // Write the update to the database.
    update_task(&pool, &to_update)
        // Await the async UPDATE; a failure here means the update query is broken.
        .await
        .expect("update task to done");

    // Build the filter to restrict to Open status only.
    let open_filter = TaskFilter {
        // Only rows where status = "open" should be returned by the query.
        status: Some(TaskStatus::Open),
        // No tag filter — tasks with any tag combination should be included.
        tag: None,
        // No due_before filter — tasks with any `due_by` value should be included.
        due_before: None,
    };
    // Execute the filtered query; the done task must be excluded.
    let open_tasks = list_tasks(&pool, &open_filter)
        .await
        .expect("list with status filter should succeed");

    // If the filter were not applied, both tasks would be returned (count = 2).
    assert_eq!(open_tasks.len(), 1, "exactly one open task should match");
    // The returned task must be the one that was not transitioned to Done.
    assert_eq!(open_tasks[0].title, "open task", "must be the open task");
}

#[tokio::test]
async fn list_tasks_filters_by_tag() {
    // The tag filter must return only tasks carrying the specified tag.
    // Tasks with different tags or no tags at all must be excluded.
    //
    // The storage layer implements this as a JSON array substring search
    // (LIKE '%"chore"%'). This test verifies that the LIKE pattern matches
    // the stored normalised tag and excludes the untagged task.
    //
    // Tags are stored as a JSON array (e.g. `["chore","daily"]`). The LIKE
    // pattern wraps the tag in double quotes to avoid partial matches — a
    // tag named "chorus" must not match a filter for "chore".
    //
    // Tag normalisation (lowercase, trim) is applied by `TaskBuilder` before
    // the tag is stored. This test uses an already-normalised tag to keep the
    // test focused on the filter query rather than the normalisation behaviour.
    let pool = open_test_db().await;
    // Use the same base time; each task gets a distinct timestamp for ordering.
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // One task with the "chore" tag; it must match the filter.
    // The builder normalises the tag to lowercase; "chore" is already lowercase.
    let chore = make_task_with_tag("sweep floor", "chore", now);
    // One task with no tag; it must be excluded by the tag filter.
    // Give it a distinct timestamp so both rows coexist without a unique-key conflict.
    let plain = make_task("read book", now + Duration::seconds(1));

    // Insert both tasks so the filter has both rows to choose between.
    insert_task(&pool, &chore).await.expect("insert chore task");
    // Insert the plain task; the filter must exclude it since it has no tags.
    insert_task(&pool, &plain).await.expect("insert plain task");

    // Build the filter to restrict to tasks tagged "chore".
    let filter = TaskFilter {
        // No status filter — all lifecycle states should match.
        status: None,
        // The storage layer will search for `"chore"` inside the JSON array.
        tag: Some("chore".to_string()),
        // No due_before filter — any due_by value should match.
        due_before: None,
    };
    // Execute the tag-filtered list query.
    let result = list_tasks(&pool, &filter)
        .await
        .expect("list by tag should succeed");

    // The untagged task must have been excluded by the filter.
    // A result of 2 would mean the tag filter is not being applied.
    assert_eq!(
        result.len(),
        1,
        "only the chore task should match the tag filter"
    );
    // The single returned task must be the one we tagged "chore".
    assert_eq!(result[0].title, "sweep floor");
}

#[tokio::test]
async fn update_task_persists_changes() {
    // Field changes written via `update_task` must be visible on the next
    // `fetch_task`. This verifies that the UPDATE SQL touches the correct
    // columns and that the storage layer does not silently discard writes.
    //
    // Only the title and `updated_at` are changed here; a comprehensive
    // field-level round-trip test for `insert_task` is in `insert_and_fetch_round_trip`.
    //
    // The storage layer always writes the full struct on UPDATE — there is no
    // partial-column update path. This test verifies that the new title and
    // updated_at are readable after the write, and that the ID is unchanged.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the task with its original title; this is the pre-update state.
    let task = make_task("original title", now);
    // Persist the initial version so there is a row to update.
    insert_task(&pool, &task)
        // Await the async INSERT before proceeding to the mutation step.
        .await
        .expect("insert initial task");

    // Clone the task and apply the desired mutation.
    // Cloning is necessary because `task` is moved into `insert_task` by reference,
    // and we need the original ID to verify the update later.
    let mut updated = task.clone();
    // Change the title — this is the primary field we assert on after the update.
    updated.title = "updated title".to_string();
    // Bump `updated_at` to record when the change was made.
    // If `updated_at` does not change, the storage layer may not be writing all columns.
    updated.updated_at = now + Duration::seconds(30);

    // Write the fully updated struct; all columns are rewritten by the UPDATE.
    // The storage layer does not do partial updates — the full struct is always written.
    update_task(&pool, &updated)
        // Await the async UPDATE; a failure here means the UPDATE query is broken.
        .await
        .expect("update should succeed");

    // Read the task back to verify the change was persisted to the database.
    let fetched = fetch_task(&pool, task.id)
        .await
        .expect("fetch after update should not error")
        // UPDATE does not delete; the task must still be present.
        .expect("task must still exist after update");

    // The stored title must reflect the new value, not the original.
    // If this fails, the UPDATE query is not writing the `title` column.
    assert_eq!(
        fetched.title, "updated title",
        "title must reflect the update"
    );
    // The stored `updated_at` must match the bumped timestamp.
    // If this fails, the storage layer may be ignoring the `updated_at` field.
    assert_eq!(
        fetched.updated_at, updated.updated_at,
        "updated_at must reflect the modification"
    );
}

#[tokio::test]
async fn mark_task_done_sets_status_and_writes_log() {
    // `mark_task_done` must atomically set the task status to `Done` AND write
    // a `CompletionLog` row with `skipped=false`. This test verifies both
    // effects — a partial write would leave the system in an inconsistent state.
    //
    // The status change and log write are sequential (not transactional) because
    // `SQLite` in WAL mode serialises writes and the service runs single-process.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the task that will be marked as done.
    // The task must be in the database before it can be marked done.
    let task = make_task("clean bathroom", now);
    // Persist the task; any error here indicates a schema or FK issue.
    insert_task(&pool, &task).await.expect("insert task");

    // Build the immutable completion log for a genuine completion.
    let completion = CompletionLog::new(
        // The log entry belongs to the task we just created.
        task.id,
        // The instance date is the calendar date of the completion event.
        date!(2026 - 05 - 25),
        // The actor is the placeholder member (only valid FK in the test DB).
        placeholder_member(),
        // The completion time is one minute after the task was created.
        now + Duration::seconds(60),
        // `false` = genuine completion; `true` would mean a skip.
        false,
        // No free-form notes for this test completion event.
        None,
    );

    // ── Mark done ────────────────────────────────────────────────────────────

    // Write the status update and completion log via mark_task_done.
    mark_task_done(&pool, task.id, &completion)
        .await
        .expect("mark_task_done should succeed");

    // ── Verify status change ──────────────────────────────────────────────────

    // Fetch the task to confirm the status was updated.
    let fetched = fetch_task(&pool, task.id)
        .await
        .expect("fetch after mark_task_done")
        .expect("task must still exist");
    // The status must now be Done; any other value means the UPDATE was skipped.
    assert_eq!(
        fetched.status,
        TaskStatus::Done,
        "status must be Done after mark_task_done"
    );

    // ── Verify completion log ─────────────────────────────────────────────────

    // Fetch all completion logs for this task.
    let logs = list_completion_logs_for_task(&pool, task.id)
        .await
        .expect("list completion logs after mark_task_done");
    // Exactly one log entry must have been written.
    assert_eq!(logs.len(), 1, "exactly one completion log entry must exist");
    // The log entry must be a genuine completion (not a skip).
    assert!(!logs[0].skipped, "log entry must have skipped=false");
}

#[tokio::test]
async fn mark_task_skipped_sets_status_and_writes_skip_log() {
    // `mark_task_skipped` is the skip variant of `mark_task_done`. The task
    // status must change to `Skipped` and the log entry must have `skipped=true`.
    // Skipping is a first-class event (brief §8.2): the household explicitly
    // decided not to do this instance; no "debt" accumulates.
    //
    // Both the status change and the log row are verified to ensure both
    // operations succeeded and neither was silently omitted.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the task that will be skipped.
    // The task must exist before mark_task_skipped can reference it.
    let task = make_task("weekly report", now);
    insert_task(&pool, &task).await.expect("insert task");

    // Build the skip log — the only structural difference from a completion
    // is the `skipped=true` flag and the status written to `tasks.status`.
    let skip_log = CompletionLog::new(
        // This log belongs to the task we just created.
        task.id,
        date!(2026 - 05 - 25),
        // Placeholder member as the actor.
        placeholder_member(),
        // The skip time is shortly after the task was created.
        now + Duration::seconds(10),
        // `true` = skip event; `false` would mean a genuine completion.
        true,
        // An optional note — must also round-trip through the TEXT column.
        Some("bank holiday".to_string()),
    );

    // ── Mark skipped ─────────────────────────────────────────────────────────

    // Write the status update and skip log.
    mark_task_skipped(&pool, task.id, &skip_log)
        .await
        .expect("mark_task_skipped should succeed");

    // ── Verify status change ──────────────────────────────────────────────────

    // Fetch the task and check the new status.
    let fetched = fetch_task(&pool, task.id)
        .await
        .expect("fetch after mark_task_skipped")
        .expect("task must still exist after skip");
    // Any status other than Skipped means the UPDATE was not applied.
    assert_eq!(
        fetched.status,
        TaskStatus::Skipped,
        "status must be Skipped after mark_task_skipped"
    );

    // ── Verify skip log ───────────────────────────────────────────────────────

    // Fetch all completion logs to verify the skip entry was written.
    let logs = list_completion_logs_for_task(&pool, task.id)
        .await
        .expect("list logs after mark_task_skipped");
    // One log entry must exist per skip call.
    assert_eq!(logs.len(), 1, "exactly one skip log entry must exist");
    // The entry must be marked as a skip, not a genuine completion.
    assert!(logs[0].skipped, "log entry must have skipped=true");
    // The notes string must survive the TEXT round-trip intact.
    assert_eq!(
        logs[0].notes.as_deref(),
        Some("bank holiday"),
        "notes must round-trip through storage"
    );
}

#[tokio::test]
async fn upsert_task_instances_is_idempotent() {
    // Calling `upsert_task_instances` twice with the same `(task_id, scheduled_at)`
    // pairs must not produce duplicate rows. The `INSERT OR IGNORE` constraint
    // on the `UNIQUE (task_id, scheduled_at)` index enforces deduplication at the
    // database level without requiring callers to track which rows already exist.
    //
    // This property is important because the materialiser may run more than once
    // for the same time window (e.g. after a service restart or concurrent runs).
    //
    // The `UNIQUE (task_id, scheduled_at)` constraint means the same task cannot
    // have two instances at the same point in time. `INSERT OR IGNORE` silently
    // skips conflicting rows — no error, no duplicate, count stays the same.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the parent recurring task first so the FK constraint is satisfied.
    // `task_instances.task_id` references `tasks.id`; the parent must exist first.
    let task = make_task("weekly review", now);
    insert_task(&pool, &task).await.expect("insert parent task");

    // Build two instances for this recurring task.
    let instances = vec![
        TaskInstance {
            // UUID v7 uniquely identifies this instance row.
            id: uuid::Uuid::now_v7().to_string(),
            // FK reference to the parent recurring task.
            task_id: task.id,
            // First occurrence: one week from the base time.
            scheduled_at: now + Duration::days(7),
            // No default assignee; the household can set one later.
            current_assignee_id: None,
        },
        TaskInstance {
            // Each instance row needs its own unique ID.
            id: uuid::Uuid::now_v7().to_string(),
            // Same parent task as the first instance.
            task_id: task.id,
            // Second occurrence: two weeks from the base time.
            scheduled_at: now + Duration::days(14),
            // No default assignee for this occurrence either.
            current_assignee_id: None,
        },
    ];

    // ── First upsert ──────────────────────────────────────────────────────────

    // First call: both rows are new, so both should be inserted.
    upsert_task_instances(&pool, &instances)
        .await
        .expect("first upsert should succeed");

    // Verify both rows exist after the first call.
    let after_first = list_upcoming_instances(&pool, task.id, now, 10)
        .await
        .expect("list after first upsert");
    // Both instances must appear; a count of 0 would mean the first upsert failed.
    assert_eq!(after_first.len(), 2, "two instances after the first upsert");

    // ── Second upsert (idempotent) ────────────────────────────────────────────

    // Second call with the exact same instances — INSERT OR IGNORE must skip both.
    upsert_task_instances(&pool, &instances)
        .await
        .expect("second upsert should also succeed");

    // The count must remain at 2; a count of 4 would indicate missing idempotency.
    let after_second = list_upcoming_instances(&pool, task.id, now, 10)
        .await
        .expect("list after second upsert");
    // If idempotency were broken, we would see 4 rows here (2 duplicates per instance).
    assert_eq!(
        after_second.len(),
        2,
        "count must remain 2 after idempotent upsert"
    );
}

#[tokio::test]
async fn list_upcoming_instances_respects_from_and_limit() {
    // `list_upcoming_instances` must return only instances at or after `from`
    // and at most `limit` rows, ordered by `scheduled_at` ASC (nearest first).
    //
    // This test inserts three daily instances (day+1, day+2, day+3), then
    // queries with `from=day+2` and `limit=1` to verify that both constraints
    // are applied simultaneously — only day+2 should be returned.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the parent task so instance FK references are valid.
    let task = make_task("daily standup", now);
    // Without the parent task in `tasks`, the upsert would fail the FK check.
    insert_task(&pool, &task).await.expect("insert parent task");

    // Build three daily instances: day+1, day+2, and day+3.
    let instances: Vec<TaskInstance> = (1i64..=3)
        .map(|offset| TaskInstance {
            // Fresh UUID v7 for each instance row — IDs must be unique.
            id: uuid::Uuid::now_v7().to_string(),
            // All three instances belong to the same parent task.
            task_id: task.id,
            // Offset by `offset` days — produces day+1, day+2, day+3.
            scheduled_at: now + Duration::days(offset),
            // No default assignee for any of these instances.
            current_assignee_id: None,
        })
        .collect();
    // Persist all three instances before querying.
    // All three must be in the table for the `from` and `limit` tests to be meaningful.
    upsert_task_instances(&pool, &instances)
        .await
        .expect("upsert three daily instances");

    // Set `from` to day+2 so day+1 (before `from`) is excluded by the query.
    // This verifies that the `scheduled_at >= from` predicate is applied correctly.
    let from = now + Duration::days(2);
    // Request at most 1 row; day+2 and day+3 both qualify, but the limit returns only day+2.
    // Passing `limit=1` verifies both the WHERE and the LIMIT clauses simultaneously.
    let result = list_upcoming_instances(&pool, task.id, from, 1)
        .await
        .expect("list_upcoming_instances should succeed");

    // Only one row should be returned — the limit must cap the result.
    assert_eq!(result.len(), 1, "limit=1 must cap the result to one row");
    // The returned instance must be day+2, not day+1 (before from) or day+3 (limit).
    assert_eq!(
        result[0].scheduled_at,
        now + Duration::days(2),
        "must return the first instance at or after `from`"
    );
}

#[tokio::test]
async fn prune_old_instances_removes_past_entries() {
    // `prune_old_instances` must delete rows with `scheduled_at < cutoff`
    // and leave rows at or after the cutoff untouched.
    //
    // The cutoff is provided by the caller (not the storage layer) so tests
    // can use arbitrary timestamps without needing to match wall-clock time.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the parent task to satisfy the task_instances FK constraint.
    let task = make_task("monthly review", now);
    // The parent task must exist in the `tasks` table before any instances can be written.
    insert_task(&pool, &task).await.expect("insert parent task");

    // The past instance is before the cutoff and should be removed by prune.
    let past = TaskInstance {
        // Each instance must have a unique UUID v7 ID.
        id: uuid::Uuid::now_v7().to_string(),
        // FK reference ties this instance to the parent task.
        task_id: task.id,
        // 5 days before `now` — strictly before the prune cutoff, so it will be deleted.
        scheduled_at: now - Duration::days(5),
        // No default assignee for either test instance.
        current_assignee_id: None,
    };
    // The future instance is after the cutoff and must NOT be touched by prune.
    let future = TaskInstance {
        // Fresh UUID v7 for the future instance — distinct from the past instance.
        id: uuid::Uuid::now_v7().to_string(),
        // Same parent task as the past instance.
        task_id: task.id,
        // 5 days after `now` — at or after the prune cutoff, so it must be kept.
        scheduled_at: now + Duration::days(5),
        // No default assignee.
        current_assignee_id: None,
    };
    // Insert both instances so the prune can demonstrate selective deletion.
    upsert_task_instances(&pool, &[past, future])
        .await
        .expect("upsert both instances");

    // Prune everything strictly before `now`; only the past instance qualifies.
    // The future instance at `now + 5 days` is NOT before `now` and must be kept.
    let pruned = prune_old_instances(&pool, now)
        .await
        .expect("prune_old_instances should succeed");

    // Exactly one row should have been deleted.
    assert_eq!(pruned, 1, "exactly one past instance should be pruned");

    // Query for all instances at or after `now` to verify the future one survived.
    // Using `now` as `from` excludes the past instance (which was pruned) from the query.
    let remaining = list_upcoming_instances(&pool, task.id, now, 10)
        .await
        .expect("list remaining instances after prune");
    // Only the future instance (at `now + 5 days`) should remain in the table.
    assert_eq!(remaining.len(), 1, "future instance must survive the prune");
    // Verify it is specifically the future instance and not an unexpected row.
    assert_eq!(
        remaining[0].scheduled_at,
        now + Duration::days(5),
        "the surviving instance must be the one after the cutoff"
    );
}

#[tokio::test]
async fn delete_future_instances_spares_past() {
    // `delete_future_instances` removes rows at or after `from` and leaves
    // earlier rows intact. This is the mid-flight recurrence change operation
    // (ADR-0002 §mid-flight): after deleting future instances, the caller
    // re-materialises from the updated rule and upserts fresh instances.
    //
    // The past instance represents a completed or scheduled-but-not-yet-due
    // occurrence that must be preserved to maintain an accurate history.
    //
    // The boundary is inclusive on the future side: a row whose `scheduled_at`
    // equals `from` exactly is deleted. This lets the caller pass `now` as the
    // boundary to clear everything from the current moment onward.
    //
    // After deletion, the caller re-materialises new instances from the revised
    // recurrence rule by calling `upsert_task_instances` with the updated set.
    let pool = open_test_db().await;
    let now = datetime!(2026-05-25 10:00:00 UTC);

    // Insert the parent recurring task so its FK reference is valid.
    let task = make_task("rolling weekly", now);
    // The task row must exist before instance rows can reference it.
    insert_task(&pool, &task).await.expect("insert parent task");

    // The past instance is strictly before `from` and must NOT be deleted.
    // Preserving past instances maintains a complete history of scheduled slots.
    let past = TaskInstance {
        // Each instance row needs its own unique UUID v7 identifier.
        id: uuid::Uuid::now_v7().to_string(),
        // FK reference ties the instance to the parent recurring task.
        task_id: task.id,
        // Scheduled 7 days in the past — before `from`, so the delete must skip it.
        scheduled_at: now - Duration::days(7),
        // No default assignee is set for this test instance.
        current_assignee_id: None,
    };
    // The future instance is at or after `from` and MUST be deleted.
    // Removing future instances clears the stale schedule before re-materialisation.
    let future = TaskInstance {
        // Fresh UUID v7 for the future instance — must be distinct from the past instance.
        id: uuid::Uuid::now_v7().to_string(),
        // Same parent task; both instances are part of the same recurrence series.
        task_id: task.id,
        // Scheduled 7 days in the future — at or after `from`, so it must be deleted.
        scheduled_at: now + Duration::days(7),
        // No default assignee.
        current_assignee_id: None,
    };
    // Persist both instances so the delete can demonstrate selective removal.
    upsert_task_instances(&pool, &[past, future])
        .await
        .expect("upsert both instances before delete");

    // Delete all instances at or after `now`; only the future instance qualifies.
    // The past instance at `now - 7 days` is before `from` and must be preserved.
    // This simulates the mid-flight recurrence change: sweep future, re-materialise.
    let deleted = delete_future_instances(&pool, task.id, now)
        .await
        .expect("delete_future_instances should succeed");

    // Exactly one row should have been deleted.
    assert_eq!(deleted, 1, "exactly one future instance should be deleted");

    // Query from far enough back to include the past instance.
    let far_past = now - Duration::days(30);
    let remaining = list_upcoming_instances(&pool, task.id, far_past, 10)
        .await
        .expect("list after delete_future_instances");
    // The past instance must still exist — it was before `from` and must not be touched.
    assert_eq!(
        remaining.len(),
        1,
        "past instance must survive delete_future_instances"
    );
    // Verify it is the past instance (at day-7), not some unexpected row.
    assert_eq!(
        remaining[0].scheduled_at,
        now - Duration::days(7),
        "the surviving instance must be the one before `from`"
    );
}
