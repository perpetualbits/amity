// completion_log.rs — repository functions for the CompletionLog entity.
//
// This module is the storage layer's interface for CompletionLog records.
// CompletionLog is append-only — there is no update or delete. The only
// public functions are:
//   • insert_completion_log — write a new log entry
//   • list_completion_logs_for_task  — ordered history for one task
//   • list_completion_logs_for_member — ordered history for one member
//
// CompletionLog rows are written by `crate::task::mark_task_done` and
// `crate::task::mark_task_skipped` as part of those operations. Direct
// callers should be rare; most code goes through the task module.
//
// Immutability invariant (brief §6.5, §8.2):
//   Rows are never updated. If a completion was logged in error, the correct
//   action is to add a note or add a new compensating row — not to edit or
//   delete the existing one. This module does not expose any mutation path.
//
// All fields use TEXT storage as per migration 0002:
//   id, task_id, completed_by → UUID TEXT
//   instance_date             → YYYY-MM-DD TEXT (time::Date)
//   completed_at              → RFC 3339 TEXT with offset
//   skipped                   → INTEGER 0/1 (SQLite STRICT mode boolean)
//   notes                     → TEXT NULL

// `SqlitePool` is the shared async connection pool for all queries in this module.
use sqlx::SqlitePool;
// `Date` for the instance_date calendar field; `OffsetDateTime` for the completion timestamp.
use time::Date;
use time::OffsetDateTime;
// RFC 3339 is the TEXT serialisation format used for all timestamp columns.
use time::format_description::well_known::Rfc3339;

// `CompletionLog` is the domain type reconstructed from / written to rows here.
use amity_core::completion_log::CompletionLog;
// `MemberId` and `TaskId` for the filter parameters on list functions.
use amity_core::ids::{MemberId, TaskId};

// `StorageError` wraps both database errors and field-parse errors.
use crate::StorageError;

// ─── Row type ────────────────────────────────────────────────────────────────

/// Raw database row from the `completion_logs` table.
///
/// All fields use SQLite-native types: TEXT for IDs/datetimes/dates, INTEGER
/// for the boolean `skipped` column. Conversion to domain types happens in
/// `row_to_completion_log`. This type is private to this module — callers only
/// see `CompletionLog` domain values.
///
/// The `#[derive(sqlx::FromRow)]` derive maps column names to field names
/// directly; field names must match the SQL column names in the SELECT queries.
#[derive(sqlx::FromRow)]
struct CompletionLogRow {
    // UUID v7 TEXT — unique ID for this log entry.
    id: String,
    // UUID TEXT — which task was completed or skipped.
    task_id: String,
    // YYYY-MM-DD TEXT — the calendar date of the scheduled instance.
    instance_date: String,
    // UUID TEXT — who performed the completion or skip action.
    completed_by: String,
    // RFC 3339 TEXT — precise timestamp with timezone offset.
    completed_at: String,
    // INTEGER 0 or 1 — 1 means this was a skip event, 0 means completion.
    skipped: i64,
    // Optional free-form context from the member; NULL when not provided.
    notes: Option<String>,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new completion log entry into the database.
///
/// This is an append-only write — there is no corresponding update or delete.
/// The `CompletionLog.id` field must already be set to a fresh UUID v7 (done
/// by `CompletionLog::new`).
///
/// Both genuine completions and skip events are inserted via this function.
/// The `skipped` field on the log entry distinguishes them — `true` for a skip
/// event, `false` for a genuine completion. There is no separate "skip" table.
///
/// The `completed_at` timestamp includes the caller's local UTC offset so that
/// the household can see the wall-clock time alongside the UTC representation.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure (including a duplicate
/// primary key if the same `CompletionLog.id` is inserted twice — should never
/// happen given UUID v7 uniqueness, but the database constraint catches it).
/// Returns `StorageError::Parse` if a field cannot be serialised.
pub async fn insert_completion_log(
    pool: &SqlitePool,
    log: &CompletionLog,
) -> Result<(), StorageError> {
    // Serialise each field to its TEXT/INTEGER storage form.
    let id = log.id.to_string();
    let task_id = log.task_id.to_string();
    // instance_date is stored as YYYY-MM-DD plain text — no timezone, just a date.
    let instance_date = format_date(log.instance_date);
    let completed_by = log.completed_by.to_string();
    // completed_at includes the timezone offset (RFC 3339 with offset).
    let completed_at = log
        .completed_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    // skipped is stored as INTEGER 0/1 in SQLite STRICT mode.
    let skipped: i64 = i64::from(log.skipped);

    sqlx::query(
        "
        INSERT INTO completion_logs
            (id, task_id, instance_date, completed_by, completed_at, skipped, notes)
        VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
    )
    .bind(id)
    .bind(task_id)
    .bind(instance_date)
    .bind(completed_by)
    .bind(completed_at)
    .bind(skipped)
    .bind(&log.notes)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all completion log entries for a task, newest first.
///
/// Returns an empty Vec when the task has no completion history. The ordering
/// is `completed_at DESC` so the most recent events appear first in the
/// household's task history view (brief §8.2).
///
/// The result includes both completions (`skipped=false`) and skip events
/// (`skipped=true`). Callers may filter by `skipped` if they need only one kind,
/// but displaying the full mixed history is the intended use case.
///
/// # Errors
///
/// Returns `StorageError` on database or parse failures.
pub async fn list_completion_logs_for_task(
    pool: &SqlitePool,
    task_id: TaskId,
) -> Result<Vec<CompletionLog>, StorageError> {
    let task_id_str = task_id.to_string();

    let rows: Vec<CompletionLogRow> = sqlx::query_as(
        "
        SELECT id, task_id, instance_date, completed_by, completed_at, skipped, notes
        FROM   completion_logs
        WHERE  task_id = ?1
        ORDER  BY completed_at DESC
        ",
    )
    .bind(task_id_str)
    .fetch_all(pool)
    .await?;

    // Parse each row; stop at the first error rather than silently skipping.
    rows.into_iter().map(row_to_completion_log).collect()
}

/// List all completion log entries for a member, newest first.
///
/// Returns the full history of what this member has done or skipped. The order
/// is `completed_at DESC`. This query supports the "who has been doing what"
/// view (brief §8.2) — the system displays facts; humans interpret them.
///
/// The result crosses all tasks — it is a member-centric view rather than a
/// task-centric one. Use `list_completion_logs_for_task` for the single-task
/// history that backs the task detail screen.
///
/// # Errors
///
/// Returns `StorageError` on database or parse failures.
pub async fn list_completion_logs_for_member(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Vec<CompletionLog>, StorageError> {
    let member_id_str = member_id.to_string();

    let rows: Vec<CompletionLogRow> = sqlx::query_as(
        "
        SELECT id, task_id, instance_date, completed_by, completed_at, skipped, notes
        FROM   completion_logs
        WHERE  completed_by = ?1
        ORDER  BY completed_at DESC
        ",
    )
    .bind(member_id_str)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_completion_log).collect()
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Convert a raw database row into a `CompletionLog` domain type.
///
/// Parses each TEXT/INTEGER field back to its domain type. Returns the first
/// parse error encountered; all fields must be valid for the row to be usable.
/// A parse error indicates either a schema mismatch or data corruption.
fn row_to_completion_log(row: CompletionLogRow) -> Result<CompletionLog, StorageError> {
    // Parse the primary key UUID.
    let id = row
        .id
        .parse::<amity_core::ids::CompletionLogId>()
        .map_err(|e| StorageError::Parse(format!("completion_log id: {e}")))?;

    // Parse the task reference UUID.
    let task_id = row
        .task_id
        .parse::<TaskId>()
        .map_err(|e| StorageError::Parse(format!("task_id: {e}")))?;

    // Parse the YYYY-MM-DD date string into a `time::Date`.
    let instance_date = parse_date(&row.instance_date)?;

    // Parse the member UUID.
    let completed_by = row
        .completed_by
        .parse::<MemberId>()
        .map_err(|e| StorageError::Parse(format!("completed_by: {e}")))?;

    // Parse the RFC 3339 timestamp (includes timezone offset).
    let completed_at = OffsetDateTime::parse(&row.completed_at, &Rfc3339)
        .map_err(|e| StorageError::Parse(format!("completed_at: {e}")))?;

    // Convert the INTEGER 0/1 to bool. SQLite STRICT mode stores booleans as
    // INTEGER; any non-zero value is treated as true per SQLite convention.
    let skipped = row.skipped != 0;

    // Reconstruct the domain type. All fields have been validated and converted
    // above; direct struct construction is used instead of CompletionLog::new
    // because we have the real id from the database, not a fresh generated one.
    Ok(CompletionLog {
        id,
        task_id,
        instance_date,
        completed_by,
        completed_at,
        skipped,
        notes: row.notes,
    })
}

/// Format a `time::Date` as a YYYY-MM-DD string for `SQLite` storage.
///
/// The format matches `SQLite`'s recommended date string format and is compatible
/// with `SQLite`'s built-in date functions (e.g. `date(instance_date, '+7 days')`).
fn format_date(date: Date) -> String {
    // Use the Display format of time::Date — it produces YYYY-MM-DD.
    // This is the canonical format for the `instance_date` column.
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

/// Parse a YYYY-MM-DD string from the database into a `time::Date`.
///
/// Returns `StorageError::Parse` with the bad input embedded in the message so
/// that operators can identify which row caused the problem from the log output.
fn parse_date(s: &str) -> Result<Date, StorageError> {
    // Use time's built-in parser for the YYYY-MM-DD format.
    let fmt = time::format_description::parse("[year]-[month]-[day]")
        .expect("date format is a compile-time constant; parse cannot fail");
    Date::parse(s, &fmt).map_err(|e| StorageError::Parse(format!("instance_date '{s}': {e}")))
}
