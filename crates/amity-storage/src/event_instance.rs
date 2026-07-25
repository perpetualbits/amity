// event_instance.rs — repository functions for the event_instances table.
//
// Materialised occurrences of recurring events, mirroring task_instances but
// without an assignee (events are not assigned). Exposes:
//   • upsert_event_instances        — bulk INSERT OR IGNORE (idempotent)
//   • list_upcoming_event_instances — next N instances for an event, from a time
//   • prune_old_event_instances     — delete instances older than a cutoff
//   • delete_future_event_instances — delete instances at/after a point in time
//                                      (used when a recurrence rule changes)
//
// The materialiser that produces the scheduled times lives in
// `amity_core::recurrence_materialiser`; this module only persists them.
//
// Notes:
//   • Events have no assignee, so an instance row is just (id, event_id,
//     scheduled_at) — simpler than a task instance.
//   • `upsert` is idempotent via INSERT OR IGNORE against the
//     (event_id, scheduled_at) UNIQUE constraint, so re-running the future
//     materialisation for an overlapping window costs nothing.
//   • Datetimes are RFC 3339 TEXT, so `scheduled_at >= ?` and `ORDER BY` work
//     without a parse step in the query engine.

// `SqlitePool` is the shared pool injected into all async queries.
use sqlx::SqlitePool;
// `OffsetDateTime` is the canonical datetime; RFC 3339 is the storage format.
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// `EventId` is the parent-event reference.
use amity_core::ids::EventId;

// `StorageError` wraps sqlx and parse failures.
use crate::StorageError;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A materialised instance of a recurring event, ready for storage.
///
/// A storage-layer type, not a domain type — it carries just enough to populate
/// an `event_instances` row. The caller generates `id` (a UUID v7 string). It
/// has no assignee: events are not "whose turn", they simply occur, which is why
/// this is thinner than the task-instance equivalent.
#[derive(Debug, Clone)]
pub struct EventInstance {
    /// UUID v7 for this instance row.
    pub id: String,
    /// The parent recurring event.
    pub event_id: EventId,
    /// When this instance is scheduled; must include a timezone offset.
    pub scheduled_at: OffsetDateTime,
}

/// Raw database row from `event_instances`. Field names match the SQL columns
/// so `sqlx::FromRow` maps them without a rename annotation.
#[derive(sqlx::FromRow)]
struct EventInstanceRow {
    // UUID v7 TEXT — instance id.
    id: String,
    // UUID TEXT — parent event.
    event_id: String,
    // RFC 3339 TEXT — scheduled time.
    scheduled_at: String,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Bulk-insert materialised event instances (idempotent).
///
/// Uses `INSERT OR IGNORE` so re-materialising the same window is a no-op — the
/// `(event_id, scheduled_at)` UNIQUE constraint dedupes at the database level.
/// The caller supplies each row's `id` (a fresh UUID v7 string); this function
/// generates nothing.
///
/// # Errors
///
/// Returns `StorageError::Database` on FK/type failures, `StorageError::Parse`
/// if a timestamp cannot be formatted.
pub async fn upsert_event_instances(
    pool: &SqlitePool,
    instances: &[EventInstance],
) -> Result<(), StorageError> {
    // Insert one at a time; batching is unnecessary at household scale and keeps
    // error reporting clear.
    for instance in instances {
        // Serialise the scheduled time and the parent id for binding.
        let scheduled = instance
            .scheduled_at
            .format(&Rfc3339)
            .map_err(|e| StorageError::Parse(e.to_string()))?;
        let event_id = instance.event_id.to_string();

        // Skip duplicates silently via INSERT OR IGNORE.
        sqlx::query(
            "
            INSERT OR IGNORE INTO event_instances (id, event_id, scheduled_at)
            VALUES (?1, ?2, ?3)
            ",
        )
        // ?1 instance id, ?2 parent event, ?3 scheduled time.
        .bind(&instance.id)
        .bind(event_id)
        .bind(scheduled)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// List the next N instances for an event, at or after `from`, ordered ascending.
///
/// Passing `from` as a day's lower bound gives the surfacing query's per-event
/// instance list; the surfacing layer filters the returned rows down to the
/// exact target day. `limit` bounds the result set so a runaway recurrence
/// cannot return an unbounded list.
///
/// # Errors
///
/// Returns `StorageError` on database or parse failure.
pub async fn list_upcoming_event_instances(
    pool: &SqlitePool,
    event_id: EventId,
    from: OffsetDateTime,
    limit: i64,
) -> Result<Vec<EventInstance>, StorageError> {
    // Format the parent id and the lower-bound time for binding.
    // Parent event id as a hyphenated UUID string.
    let event_id_str = event_id.to_string();
    // Lower bound as RFC 3339 TEXT for the >= comparison.
    let from_str = from
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;

    // The nearest `limit` instances at or after `from`, ascending.
    let rows: Vec<EventInstanceRow> = sqlx::query_as(
        "
        SELECT id, event_id, scheduled_at
        FROM   event_instances
        WHERE  event_id = ?1 AND scheduled_at >= ?2
        ORDER  BY scheduled_at ASC
        LIMIT  ?3
        ",
    )
    // ?1 parent event, ?2 lower-bound time, ?3 row limit.
    .bind(event_id_str)
    .bind(from_str)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    // Parse each row into the storage type.
    rows.into_iter().map(row_to_instance).collect()
}

/// Delete all event instances with `scheduled_at < cutoff`.
///
/// Used by a future events horizon job to prune aged-out instances (the event
/// counterpart to task pruning). Safe because event overrides reference the
/// event and its date, not the instance row — so pruning an instance never
/// orphans an override.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn prune_old_event_instances(
    pool: &SqlitePool,
    cutoff: OffsetDateTime,
) -> Result<u64, StorageError> {
    // Format the cutoff as RFC 3339 TEXT for the < comparison.
    let cutoff_str = cutoff
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;

    // One DELETE removes every aged-out instance in the table.
    let result = sqlx::query("DELETE FROM event_instances WHERE scheduled_at < ?1")
        .bind(cutoff_str)
        .execute(pool)
        .await?;

    // Return the count removed so callers can log it.
    Ok(result.rows_affected())
}

/// Delete all instances for an event scheduled at or after `from`.
///
/// Called when a recurrence rule changes: sweep future instances, then
/// re-materialise from the new rule. Past instances (and any completed history
/// referencing them) are preserved — only the not-yet-happened slots are swept
/// so the new rule can fill them.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn delete_future_event_instances(
    pool: &SqlitePool,
    event_id: EventId,
    from: OffsetDateTime,
) -> Result<u64, StorageError> {
    // Format the parent id and the lower-bound time.
    // Parent event id.
    let event_id_str = event_id.to_string();
    // Cutoff for "future" instances.
    let from_str = from
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;

    let result =
        // Sweep this event's not-yet-happened slots only.
        sqlx::query("DELETE FROM event_instances WHERE event_id = ?1 AND scheduled_at >= ?2")
            .bind(event_id_str)
            .bind(from_str)
            .execute(pool)
            .await?;

    Ok(result.rows_affected())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Convert a raw row into an `EventInstance`.
///
/// Parses the parent-event UUID and the RFC 3339 scheduled time; the instance id
/// is stored and returned verbatim.
fn row_to_instance(row: EventInstanceRow) -> Result<EventInstance, StorageError> {
    // Parse the parent event id.
    let event_id = row
        .event_id
        .parse::<EventId>()
        .map_err(|e| StorageError::Parse(format!("event_id: {e}")))?;
    // Parse the scheduled time from RFC 3339.
    let scheduled_at = OffsetDateTime::parse(&row.scheduled_at, &Rfc3339)
        .map_err(|e| StorageError::Parse(format!("scheduled_at: {e}")))?;
    Ok(EventInstance {
        // Id stored verbatim.
        id: row.id,
        // Parsed parent-event id.
        event_id,
        // Parsed scheduled time.
        scheduled_at,
    })
}
