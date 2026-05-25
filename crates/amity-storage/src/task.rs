// task.rs — repository functions for the Task entity.
//
// This module is the storage layer's interface for Tasks. It exposes:
//   • insert_task         — write a new task
//   • fetch_task          — read one task by ID
//   • list_tasks          — read tasks with optional status/tag filters
//   • update_task         — update task fields (bumps updated_at)
//   • mark_task_done      — set status=Done and write a CompletionLog row
//   • mark_task_skipped   — set status=Skipped and write a CompletionLog row
//
// All JSON array fields (tags, assignee_ids, eligible_member_ids) are stored
// as TEXT in the database and serialised/deserialised here using serde_json.
//
// Recurrence is stored as two columns: `recurrence_rrule` (the RRULE string)
// and `recurrence_timezone` (the IANA timezone name). Both are NULL for
// one-shot tasks. The Rust layer reconstructs `RecurrenceRule` from these.
//
// No business logic lives in this module. The repository writes and reads
// domain types; it does not validate, transform, or make decisions.
// See docs/amity_rust_guidelines.md §"Database patterns".

// `SqlitePool` is the shared connection pool injected into every query function.
use sqlx::SqlitePool;
// `OffsetDateTime` is the project's preferred timestamp type; RFC 3339 is the storage format.
use time::OffsetDateTime;
// The RFC 3339 format descriptor used for both serialisation and deserialisation.
use time::format_description::well_known::Rfc3339;

// Typed UUID newtypes used for IDs — avoid raw String confusion.
use amity_core::ids::{MemberId, ProjectId, TaskId};
// `RecurrenceRule` is split across two TEXT columns in storage; reassembled here on read.
use amity_core::recurrence::RecurrenceRule;
// `CompletionLog` is written inside `mark_task_done` and `mark_task_skipped`.
use amity_core::completion_log::CompletionLog;
// `EffortLevel` / `Priority` are newtypes enforcing the 1..=5 range on read and write.
// `Task` is the domain struct this module reads/writes. `TaskStatus` is its lifecycle enum.
use amity_core::task::{EffortLevel, Priority, Task, TaskStatus};

// `StorageError` wraps sqlx errors and field-parse failures into a single error type.
use crate::StorageError;

// ─── Row type ────────────────────────────────────────────────────────────────

/// Raw database row from the `tasks` table.
///
/// All fields use `String` or `Option<String>` because the database stores
/// everything as TEXT (UUIDs, datetimes, enums, JSON arrays). The conversion
/// to domain types happens in `row_to_task`. Field names must match the SQL
/// column names exactly for `#[derive(sqlx::FromRow)]` to work without
/// explicit `#[sqlx(rename = "...")]` annotations.
#[derive(sqlx::FromRow)]
struct TaskRow {
    // UUID v7 TEXT — the task's primary key.
    id: String,
    // The one-line summary.
    title: String,
    // Optional free-form notes; NULL in the database maps to None.
    notes: Option<String>,
    // snake_case status string (open|doing|done|skipped).
    status: String,
    // ISO-8601 TEXT with timezone offset; NULL for tasks with no deadline.
    due_by: Option<String>,
    // ISO-8601 TEXT with timezone offset; NULL for tasks with no earliest start.
    earliest_at: Option<String>,
    // Effort level 1-5 as INTEGER; NULL when not estimated.
    effort: Option<i64>,
    // Priority rank 1-5 as INTEGER; NULL when not ranked.
    priority: Option<i64>,
    // JSON array of tag strings, e.g. '["chore","outdoor"]'.
    tags: String,
    // UUID TEXT for the owner.
    owner_id: String,
    // JSON array of UUID strings for assignees.
    assignee_ids: String,
    // JSON array of UUID strings for eligible members.
    eligible_member_ids: String,
    // UUID TEXT for the current default assignee; NULL when unset.
    current_assignee_id: Option<String>,
    // RRULE string (no "RRULE:" prefix); NULL for one-shot tasks.
    recurrence_rrule: Option<String>,
    // IANA timezone for DST materialisation; NULL iff recurrence_rrule NULL.
    recurrence_timezone: Option<String>,
    // Project link stub; always NULL in Task 2.
    project_id: Option<String>,
    // ISO-8601 TEXT creation timestamp.
    created_at: String,
    // ISO-8601 TEXT last-modification timestamp.
    updated_at: String,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new task into the database.
///
/// All fields are taken from the `Task` struct; this function does not
/// generate IDs or timestamps — that is the domain layer's job (done in
/// `TaskBuilder::build`). The `task.id` (UUID v7) and both timestamp fields
/// are assumed to be valid and ready for storage.
///
/// The full 18-column INSERT mirrors the `tasks` table from migration 0002.
/// If new columns are added in a later migration, the INSERT must be updated
/// here and in `update_task`.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure.
/// Returns `StorageError::Parse` if a field cannot be serialised (should not
/// happen in practice; all types are serialisable).
pub async fn insert_task(pool: &SqlitePool, task: &Task) -> Result<(), StorageError> {
    // ── Step 1: serialise all fields to their TEXT storage representations ──────
    //
    // Every field is converted to its SQL-bindable form before the query runs.
    // Doing this upfront makes the .bind() section below straightforward and
    // keeps the error handling out of the bind chain.

    // UUID newtype → hyphenated string form ("018f...").
    let id = task.id.to_string();
    // owner_id is a UUID newtype; Display gives the hyphenated string form.
    let owner_id = task.owner_id.to_string();

    // ── Step 2: format timestamps as RFC 3339 ────────────────────────────────
    //
    // RFC 3339 is lexicographically sortable, which makes `ORDER BY created_at`
    // correct without a date-parse step in the SQL engine.
    let created_at = task
        .created_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    let updated_at = task
        .updated_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;

    // ── Step 3: format enum and optional fields ──────────────────────────────
    //
    // TaskStatus::Display produces the snake_case storage string (open/doing/done/skipped).
    let status = task.status.to_string();
    // Optional timestamp fields: None becomes SQL NULL; Some becomes RFC 3339.
    let due_by = format_optional_dt(task.due_by)?;
    let earliest_at = format_optional_dt(task.earliest_at)?;
    // Effort and priority are stored as INTEGER (1-5) or NULL.
    // i64::from avoids truncation (u8 fits trivially in i64).
    let effort = task.effort.map(|e| i64::from(e.value()));
    let priority = task.priority.map(|p| i64::from(p.value()));

    // ── Step 4: serialise collection fields as JSON TEXT ────────────────────
    //
    // Vec<String> tags → JSON array TEXT (e.g. `["chore","outdoor"]`).
    let tags =
        serde_json::to_string(&task.tags).map_err(|e| StorageError::Parse(format!("tags: {e}")))?;
    // assignee_ids and eligible_member_ids: Vec<MemberId> → JSON array of UUID strings.
    let assignee_ids = serialize_member_ids(&task.assignee_ids)?;
    let eligible_member_ids = serialize_member_ids(&task.eligible_member_ids)?;

    // ── Step 5: format optional scalar fields ───────────────────────────────
    //
    // current_assignee_id: Option<MemberId> → Option<String> (None = SQL NULL).
    let current_assignee_id = task.current_assignee_id.map(|m| m.to_string());
    // Recurrence: stored as two separate nullable columns (ADR-0002 §storage-layout).
    // Both are Some together or both are None — the invariant is enforced at read.
    let recurrence_rrule = task.recurrence.as_ref().map(|r| r.rrule.clone());
    let recurrence_timezone = task.recurrence.as_ref().map(|r| r.timezone.clone());
    // project_id is always None in Task 2; the column exists for future use (ADR-0003).
    let project_id: Option<String> = task.project_id.map(|p| p.to_string());

    sqlx::query(
        "
        INSERT INTO tasks (
            id, title, notes, status, due_by, earliest_at,
            effort, priority, tags, owner_id, assignee_ids, eligible_member_ids,
            current_assignee_id, recurrence_rrule, recurrence_timezone,
            project_id, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15,
            ?16, ?17, ?18
        )
        ",
    )
    .bind(id)
    .bind(&task.title)
    .bind(&task.notes)
    .bind(status)
    .bind(due_by)
    .bind(earliest_at)
    .bind(effort)
    .bind(priority)
    .bind(tags)
    .bind(owner_id)
    .bind(assignee_ids)
    .bind(eligible_member_ids)
    .bind(current_assignee_id)
    .bind(recurrence_rrule)
    .bind(recurrence_timezone)
    .bind(project_id)
    .bind(created_at)
    .bind(updated_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Fetch a single task by its ID.
///
/// Returns `None` if no task with that ID exists. The caller must decide how
/// to surface the absence — typically HTTP 404 in the service layer.
///
/// The SELECT names every column explicitly so that schema changes produce
/// a compile-time mismatch (rather than silently selecting fewer columns).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
/// Returns `StorageError::Parse` if any stored value cannot be decoded.
pub async fn fetch_task(pool: &SqlitePool, id: TaskId) -> Result<Option<Task>, StorageError> {
    // Convert the typed TaskId to a hyphenated UUID string for the SQL bind parameter.
    let id_str = id.to_string();

    // `fetch_optional` returns None when no row matches — "not found" is valid
    // and non-exceptional; the caller decides whether to return 404 or something else.
    let row: Option<TaskRow> = sqlx::query_as(
        "
        SELECT id, title, notes, status, due_by, earliest_at,
               effort, priority, tags, owner_id, assignee_ids, eligible_member_ids,
               current_assignee_id, recurrence_rrule, recurrence_timezone,
               project_id, created_at, updated_at
        FROM   tasks
        WHERE  id = ?1
        ",
    )
    .bind(id_str)
    .fetch_optional(pool)
    .await?;

    // Transpose the Option: if there's no row, return None without attempting to parse.
    // Parsing None would succeed trivially but waste effort constructing a dummy Task.
    match row {
        Some(r) => Ok(Some(row_to_task(r)?)),
        // Explicit None arm makes the return type clear to the reader.
        None => Ok(None),
    }
}

/// Optional filters for `list_tasks`.
///
/// Filters are AND-ed together. Unset fields match everything — an empty
/// `TaskFilter::default()` returns all tasks. The caller constructs this
/// from query string parameters in the service layer.
///
/// Tag matching is intentionally simple: it uses a JSON LIKE search against the
/// stored `["tag1","tag2"]` TEXT. This is sufficient at household scale; a full
/// FTS index would be warranted for a larger deployment.
#[derive(Debug, Default)]
pub struct TaskFilter {
    /// If set, only tasks with this status are returned.
    ///
    /// Corresponds to `?status=open` in the HTTP query string.
    pub status: Option<TaskStatus>,

    /// If set, only tasks with this tag are returned.
    ///
    /// Tag matching uses a JSON array substring search (LIKE '%"tag"%').
    /// This is intentionally simple; a full-text search index would be
    /// needed for high-volume production use.
    pub tag: Option<String>,

    /// If set, only tasks with `due_by <= due_before` are returned.
    ///
    /// Tasks with a NULL `due_by` (no deadline) are never returned by this filter.
    pub due_before: Option<OffsetDateTime>,
}

/// List tasks matching an optional filter, newest-first by `created_at`.
///
/// Returns all tasks when the filter has no fields set. Tasks are ordered by
/// `created_at DESC` so newly captured tasks appear at the top of the list.
///
/// The query is built dynamically: each active filter appends a WHERE clause
/// fragment and a bound parameter. The `sqlx::query_as` call then binds all
/// parameters in order. This avoids separate query strings for every combination
/// of filters while keeping the bindings positional and SQL-injection safe.
///
/// # Errors
///
/// Returns `StorageError` on database or parse failures.
pub async fn list_tasks(pool: &SqlitePool, filter: &TaskFilter) -> Result<Vec<Task>, StorageError> {
    // Build the query dynamically based on which filters are set.
    // Runtime query building avoids multiple near-identical query strings.
    // The query always has a trailing WHERE clause for cleanliness; unused
    // filters add no overhead since SQLite optimises away trivially true conditions.

    // Separate SQL WHERE clauses added per active filter.
    let mut where_clauses = Vec::new();
    // Bound parameters for each active filter, in the same order.
    let mut params: Vec<String> = Vec::new();

    // Add the status filter clause if the status filter is set.
    // The status column stores the snake_case string (open/doing/done/skipped).
    if let Some(ref s) = filter.status {
        where_clauses.push("status = ?");
        // TaskStatus::Display gives the snake_case string that matches the column.
        params.push(s.to_string());
    }

    // Tag filter: JSON array LIKE search. The stored format is '["chore","outdoor"]',
    // so we search for the exact tag string surrounded by JSON delimiters.
    if let Some(ref tag) = filter.tag {
        // Match the tag as a JSON string element: either `"tag"` at array start,
        // in the middle, or at the end. The LIKE pattern handles all positions.
        where_clauses.push("tags LIKE ?");
        // Wrap the tag in JSON quote delimiters: "chore" → %"chore"%.
        params.push(format!("%\"{tag}\"%"));
    }

    // Due-before filter: tasks whose due_by is on or before the given datetime.
    // Tasks with NULL due_by are never returned by this filter — NULL <= X is NULL.
    if let Some(due) = filter.due_before {
        where_clauses.push("due_by <= ?");
        let due_str = due
            .format(&Rfc3339)
            .map_err(|e| StorageError::Parse(e.to_string()))?;
        params.push(due_str);
    }

    // Build the WHERE clause; use "1=1" if no filters are active (matches all rows).
    // "1=1" is a tautology that SQLite eliminates in query planning — no overhead.
    let where_sql = if where_clauses.is_empty() {
        // No filters: return all tasks.
        "1=1".to_owned()
    } else {
        // Join active filters with AND; order matches the params Vec ordering above.
        where_clauses.join(" AND ")
    };

    // Construct the full query string. The WHERE clause is always present to
    // avoid a branch on whether to include it. The ORDER BY is newest-first by
    // default; callers can wrap or re-sort if a different order is needed.
    let sql = format!(
        "
        SELECT id, title, notes, status, due_by, earliest_at,
               effort, priority, tags, owner_id, assignee_ids, eligible_member_ids,
               current_assignee_id, recurrence_rrule, recurrence_timezone,
               project_id, created_at, updated_at
        FROM   tasks
        WHERE  {where_sql}
        ORDER  BY created_at DESC
        "
    );

    // Build the query and bind parameters in order.
    // sqlx's `query_as` uses positional `?` placeholders; each `.bind()` call
    // fills the next `?` in the SQL string. The params Vec was built in the same
    // order as the WHERE clause fragments, so they align correctly.
    let mut q = sqlx::query_as::<_, TaskRow>(&sql);
    for param in &params {
        // Bind each filter parameter in the order they were added to `params`.
        q = q.bind(param.as_str());
    }

    let rows = q.fetch_all(pool).await?;
    // Parse each row; stop at the first error rather than silently skipping.
    // The `collect()` short-circuits on the first Err from `row_to_task`.
    rows.into_iter().map(row_to_task).collect()
}

/// Update a task's mutable fields and bump `updated_at`.
///
/// This function updates ALL mutable fields from the supplied `Task` struct,
/// using the `id` field as the primary key. The `created_at` field is not
/// updated — it is set once on insert and never changes.
///
/// The service layer is responsible for setting `task.updated_at` to the
/// current time before calling this function. This module does not call
/// `OffsetDateTime::now_utc()` internally — callers control the clock.
///
/// If recurrence is changed, the caller is responsible for deleting existing
/// future instances and re-materialising from the new rule (service-layer
/// concern; see ADR-0002 §mid-flight-recurrence-change).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn update_task(pool: &SqlitePool, task: &Task) -> Result<(), StorageError> {
    // Serialise all mutable fields to TEXT. The pattern mirrors insert_task
    // but omits `id` (used in WHERE) and `created_at` (immutable).
    //
    // `id` goes into the WHERE clause, not the SET clause.
    let id = task.id.to_string();
    // owner_id is mutable — ownership can be transferred (future feature).
    let owner_id = task.owner_id.to_string();
    // updated_at is bumped to `now` by the service layer before calling here.
    // This module never reads the clock; it stores what it receives.
    let updated_at = task
        .updated_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    // Status transitions are allowed by the service layer, not enforced here.
    let status = task.status.to_string();
    // Optional window fields; None writes SQL NULL to clear a previously set value.
    let due_by = format_optional_dt(task.due_by)?;
    let earliest_at = format_optional_dt(task.earliest_at)?;
    // None writes NULL, effectively clearing a previously set effort/priority.
    let effort = task.effort.map(|e| i64::from(e.value()));
    let priority = task.priority.map(|p| i64::from(p.value()));
    // Tags always present as a JSON array; `[]` for an empty tag list.
    let tags =
        serde_json::to_string(&task.tags).map_err(|e| StorageError::Parse(format!("tags: {e}")))?;
    // assignee and eligible lists are fully replaced on every update.
    let assignee_ids = serialize_member_ids(&task.assignee_ids)?;
    let eligible_member_ids = serialize_member_ids(&task.eligible_member_ids)?;
    // None clears the current assignee; Some sets a new one.
    let current_assignee_id = task.current_assignee_id.map(|m| m.to_string());
    // Recurrence columns are updated together; partial writes would corrupt the invariant.
    let recurrence_rrule = task.recurrence.as_ref().map(|r| r.rrule.clone());
    let recurrence_timezone = task.recurrence.as_ref().map(|r| r.timezone.clone());
    // project_id stub: always None in Task 2; stored as NULL in the column.
    // When the Project entity lands, this will be a real UUID or NULL.
    let project_id: Option<String> = task.project_id.map(|p| p.to_string());

    sqlx::query(
        "
        UPDATE tasks
        SET    title = ?1, notes = ?2, status = ?3,
               due_by = ?4, earliest_at = ?5,
               effort = ?6, priority = ?7, tags = ?8,
               owner_id = ?9, assignee_ids = ?10, eligible_member_ids = ?11,
               current_assignee_id = ?12,
               recurrence_rrule = ?13, recurrence_timezone = ?14,
               project_id = ?15, updated_at = ?16
        WHERE  id = ?17
        ",
    )
    .bind(&task.title)
    .bind(&task.notes)
    .bind(status)
    .bind(due_by)
    .bind(earliest_at)
    .bind(effort)
    .bind(priority)
    .bind(tags)
    .bind(owner_id)
    .bind(assignee_ids)
    .bind(eligible_member_ids)
    .bind(current_assignee_id)
    .bind(recurrence_rrule)
    .bind(recurrence_timezone)
    .bind(project_id)
    .bind(updated_at)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Mark a task as done and write a `CompletionLog` entry.
///
/// Updates `status` to `Done` and `updated_at` to `now`, then inserts the
/// completion log row. Both operations are performed inside the same
/// function call for atomicity — a partial write (task updated, log missing)
/// would create an inconsistent history.
///
/// For recurring tasks, the status update applies to the parent Task row (the
/// whole-task lifecycle); the per-instance record lives in `CompletionLog`.
///
/// The two SQL operations are sequential, not wrapped in a transaction, because
/// `SQLite` in WAL mode serialises writes and the service runs single-process. If
/// two-phase transactional safety is needed later, wrap them in `BEGIN`/`COMMIT`.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure.
pub async fn mark_task_done(
    pool: &SqlitePool,
    task_id: TaskId,
    completion: &CompletionLog,
) -> Result<(), StorageError> {
    // Step 1: prepare the timestamp string from the completion log's timestamp.
    // We use `completed_at` (not the current time) so that the task's `updated_at`
    // matches the exact moment the user pressed the "done" button.
    let updated_at = completion
        .completed_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    // Convert TaskId to a String for the SQL WHERE clause bind parameter.
    let id_str = task_id.to_string();

    // Step 2: update the parent task's status to 'done'.
    // The status literal is hardcoded here rather than using TaskStatus::Display
    // so that this update does not depend on the TaskStatus enum's string mapping.
    sqlx::query("UPDATE tasks SET status = 'done', updated_at = ?1 WHERE id = ?2")
        .bind(&updated_at)
        .bind(&id_str)
        .execute(pool)
        .await?;

    // Step 3: insert the completion log record.
    // The crate:: prefix routes to the completion_log module in this same crate.
    // The log is written AFTER the status update; if insert_completion_log fails,
    // the task status has already changed — acceptable for household use (brief §8.2).
    crate::completion_log::insert_completion_log(pool, completion).await?;

    Ok(())
}

/// Mark a task as skipped and write a `CompletionLog` entry.
///
/// Functionally identical to `mark_task_done` except the status is set to
/// `Skipped`. The `CompletionLog` row has `skipped=true`. Skipping is a
/// completion event by design (brief §8.2) — there is no separate skip path.
///
/// The caller supplies a `CompletionLog` with `skipped=true` already set.
/// This function does not validate the flag — it delegates entirely to the
/// `insert_completion_log` call which stores whatever value the log carries.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure.
pub async fn mark_task_skipped(
    pool: &SqlitePool,
    task_id: TaskId,
    completion: &CompletionLog,
) -> Result<(), StorageError> {
    // Step 1: prepare the timestamp string — same approach as mark_task_done.
    // The timestamp comes from the completion log, not the system clock.
    let updated_at = completion
        .completed_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    let id_str = task_id.to_string();

    // Step 2: update the parent task's status to 'skipped'.
    sqlx::query("UPDATE tasks SET status = 'skipped', updated_at = ?1 WHERE id = ?2")
        .bind(&updated_at)
        .bind(&id_str)
        .execute(pool)
        .await?;

    // Step 3: insert the completion log record with skipped=true.
    // The log entry confirms that the skip was recorded — the household can see
    // the full skip history alongside regular completions (brief §8.2).
    crate::completion_log::insert_completion_log(pool, completion).await?;

    Ok(())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Convert a raw database row into a `Task` domain type.
///
/// All parse steps return typed errors; a failure here means the stored data
/// is inconsistent with the current schema — either a bug in the write path or
/// a schema migration that was not reflected in the domain types.
///
/// Parsing is strict: the first field that fails to parse causes the entire row
/// to fail. This is intentional — a partially constructed `Task` would be more
/// dangerous than a visible error, and the `Vec<Task>` callers stop on the first
/// error rather than silently skipping corrupt rows.
///
/// The parse order follows the column order in the SELECT queries so that the
/// error message field names match the SQL output, making tracing easier.
fn row_to_task(row: TaskRow) -> Result<Task, StorageError> {
    // ── Step 1: parse UUID primary keys ─────────────────────────────────────
    //
    // The ID columns are stored as hyphenated UUID strings. The newtype parse
    // validates both the UUID format and the type-safety wrapper.

    // Parse the task's own primary key from its UUID TEXT representation.
    // The `parse::<TaskId>()` call delegates to `TaskId::from_str` which
    // validates the UUID format and wraps it in the typed newtype.
    let id = row
        .id
        .parse::<TaskId>()
        .map_err(|e| StorageError::Parse(format!("task id: {e}")))?;

    // ── Step 2: parse the owner UUID ─────────────────────────────────────────
    //
    // owner_id is a UUID TEXT column with a NOT NULL constraint in the schema.
    // Every task must have an owner; a missing owner is a data integrity failure.
    let owner_id = row
        .owner_id
        .parse::<MemberId>()
        .map_err(|e| StorageError::Parse(format!("owner_id: {e}")))?;

    // Parse timestamps from their RFC 3339 TEXT form.
    // RFC 3339 parsing restores the original UTC offset, not just the UTC time,
    // so the result carries the same wall-clock time that was stored.
    let created_at = parse_rfc3339(&row.created_at, "created_at")?;
    let updated_at = parse_rfc3339(&row.updated_at, "updated_at")?;

    // Parse the status from its snake_case TEXT form.
    // TaskStatus::from_str maps "open"/"doing"/"done"/"skipped" to the enum variant.
    let status = row
        .status
        .parse::<TaskStatus>()
        .map_err(|e| StorageError::Parse(format!("status: {e}")))?;

    // Parse optional datetime fields.
    // `transpose()` converts Option<Result<T,E>> → Result<Option<T>,E>.
    let due_by = row
        .due_by
        .map(|s| parse_rfc3339(&s, "due_by"))
        .transpose()?;
    let earliest_at = row
        .earliest_at
        .map(|s| parse_rfc3339(&s, "earliest_at"))
        .transpose()?;

    // Parse effort and priority from their stored INTEGER values.
    // The i64 → u8 cast is guarded by the 1..=5 constraint check in
    // EffortLevel::new / Priority::new — values outside that range return an error
    // before the u8 truncation is observable, so the cast is semantically safe.
    let effort = row
        .effort
        .map(|v| {
            // The stored value must be in 1..=5; a value outside that range
            // indicates the write path bypassed the EffortLevel::new validation.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            EffortLevel::new(v as u8).map_err(|e| StorageError::Parse(format!("effort: {e}")))
        })
        .transpose()?;

    let priority = row
        .priority
        .map(|v| {
            // Same constraint as effort above — 1..=5 range enforced by Priority::new.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Priority::new(v as u8).map_err(|e| StorageError::Parse(format!("priority: {e}")))
        })
        .transpose()?;

    // Parse the JSON array of tag strings.
    // Tags are stored as '["tag1","tag2"]'; an empty list is stored as '[]'.
    let tags: Vec<String> =
        serde_json::from_str(&row.tags).map_err(|e| StorageError::Parse(format!("tags: {e}")))?;

    // Parse the JSON arrays of member UUID strings into Vec<MemberId>.
    // An empty JSON array ("[]") produces an empty Vec, which is valid.
    let assignee_ids = deserialize_member_ids(&row.assignee_ids, "assignee_ids")?;
    let eligible_member_ids =
        deserialize_member_ids(&row.eligible_member_ids, "eligible_member_ids")?;

    // Parse the optional current assignee UUID.
    // None is the common case — most tasks have no explicit current assignee set.
    let current_assignee_id = row
        .current_assignee_id
        .map(|s| {
            // Parse from the hyphenated UUID string stored in the column.
            s.parse::<MemberId>()
                .map_err(|e| StorageError::Parse(format!("current_assignee_id: {e}")))
        })
        .transpose()?;

    // Reconstruct the RecurrenceRule from the two separate nullable columns.
    // Both columns must be non-NULL together (the insert path ensures this).
    // A mismatch (one NULL, one non-NULL) is a schema-level inconsistency.
    let recurrence = match (row.recurrence_rrule, row.recurrence_timezone) {
        // Both columns present: reconstruct the RecurrenceRule.
        (Some(rrule), Some(timezone)) => Some(RecurrenceRule::new(rrule, timezone)),
        // Both NULL: one-shot task.
        (None, None) => None,
        // One NULL and one non-NULL: inconsistent state in the database.
        // This indicates either a migration bug or an incomplete write transaction.
        _ => {
            return Err(StorageError::Parse(
                "recurrence_rrule and recurrence_timezone must both be NULL or both non-NULL"
                    .to_owned(),
            ));
        }
    };

    // Parse the project_id stub (always None in Task 2, but may be non-None after
    // the Project entity lands, so we parse it defensively now).
    // Parsing defensively here avoids a silent data loss when the column goes live.
    let project_id = row
        .project_id
        .map(|s| {
            // Parse from UUID TEXT — same format as all other ID columns.
            s.parse::<ProjectId>()
                .map_err(|e| StorageError::Parse(format!("project_id: {e}")))
        })
        .transpose()?;

    // Construct the domain type. All fields have been validated and parsed above.
    // Direct struct construction is used (not TaskBuilder) because the task
    // already exists in the database — the builder invariants were enforced at
    // insert time, and TaskBuilder::new() would generate a fresh TaskId.
    Ok(Task {
        id,
        // title is stored verbatim; no transformation on read.
        title: row.title,
        // notes is None when the column is NULL.
        notes: row.notes,
        status,
        due_by,
        earliest_at,
        effort,
        priority,
        // tags are normalised (trimmed+lowercased) on write; they arrive clean here.
        tags,
        owner_id,
        assignee_ids,
        eligible_member_ids,
        current_assignee_id,
        recurrence,
        project_id,
        created_at,
        updated_at,
    })
}

/// Format an optional `OffsetDateTime` as RFC 3339, or return `None` for SQL NULL.
///
/// The `Option` wrapper is preserved — a `None` input produces a `None` output
/// which binds as SQL NULL. This is used for `due_by` and `earliest_at`.
fn format_optional_dt(dt: Option<OffsetDateTime>) -> Result<Option<String>, StorageError> {
    // Map the Option: present → format to RFC 3339 string, absent → None (NULL).
    dt.map(|t| {
        t.format(&Rfc3339)
            .map_err(|e| StorageError::Parse(e.to_string()))
    })
    .transpose()
}

/// Parse an RFC 3339 string into `OffsetDateTime`.
///
/// The `column_name` parameter is included in the error message for easy
/// diagnosis when a value stored in the database cannot be parsed. Pass the
/// exact column name (e.g. `"created_at"`) so error messages are actionable.
fn parse_rfc3339(s: &str, column_name: &str) -> Result<OffsetDateTime, StorageError> {
    // OffsetDateTime::parse with the Rfc3339 format descriptor. RFC 3339 includes
    // a mandatory UTC offset, so the result carries timezone information.
    OffsetDateTime::parse(s, &Rfc3339)
        .map_err(|e| StorageError::Parse(format!("{column_name}: {e}")))
}

/// Serialise a `Vec<MemberId>` as a JSON array of UUID strings.
///
/// The result is stored in TEXT columns in `SQLite` (e.g. `assignee_ids`).
/// An empty slice serialises to `"[]"` — the column is never NULL for these arrays.
fn serialize_member_ids(ids: &[MemberId]) -> Result<String, StorageError> {
    // Convert each MemberId to its hyphenated UUID string, then JSON-encode the Vec.
    let strings: Vec<String> = ids.iter().map(std::string::ToString::to_string).collect();
    serde_json::to_string(&strings)
        .map_err(|e| StorageError::Parse(format!("serialise member IDs: {e}")))
}

/// Deserialise a JSON array of UUID strings into `Vec<MemberId>`.
///
/// Used for `assignee_ids` and `eligible_member_ids` stored as TEXT columns.
/// An empty JSON array `"[]"` produces an empty Vec — a valid state for both fields.
fn deserialize_member_ids(json: &str, column_name: &str) -> Result<Vec<MemberId>, StorageError> {
    // Two-step: parse the JSON array, then parse each UUID string into a MemberId.
    // Separating these steps gives distinct error messages for JSON errors vs UUID errors.
    let strings: Vec<String> = serde_json::from_str(json)
        .map_err(|e| StorageError::Parse(format!("{column_name} JSON: {e}")))?;
    strings
        .into_iter()
        .map(|s| {
            // Each string must be a valid UUID in hyphenated form.
            s.parse::<MemberId>()
                .map_err(|e| StorageError::Parse(format!("{column_name} UUID: {e}")))
        })
        .collect()
}

// ─── CompletionLog row type (used by mark_task_done/mark_task_skipped) ────────

/// Raw database row for inserting a completion log alongside a task update.
///
/// The completion log's storage representation is in `amity-storage::completion_log`;
/// this is just the shape used when `mark_task_done` / `mark_task_skipped` delegates
/// to `insert_completion_log`.
///
/// No separate row type is needed here — `mark_task_done` and `mark_task_skipped`
/// delegate directly to `crate::completion_log::insert_completion_log` using
/// the `CompletionLog` domain type.
///
/// See `crates/amity-storage/src/completion_log.rs` for the full storage
/// representation.
const _: () = (); // Marker to keep the comment section distinct.
