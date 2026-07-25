// event_override.rs — repository functions for the event_overrides table.
//
// Local overlays on read-only external event instances (brief §6.5, §7). Exposes:
//   • insert_event_override  — write a new overlay (append-only)
//   • list_overrides_on_date — all overlays targeting a given calendar date,
//                              for the surfacing layer to apply
//
// The surfacing layer loads a day's overrides once and applies them while
// gathering event candidates (e.g. a `Cancel` hides its instance). The source
// feed is never modified — the change is held locally.
//
// Notes:
//   • `instance_date` is stored as YYYY-MM-DD TEXT, matching the surfacing
//     query which asks "what overrides apply on day D".
//   • The `action` enum and `created_by`/event ids are stored as strings and
//     reassembled on read, like every other entity here.

// `SqlitePool` is the shared pool.
use sqlx::SqlitePool;
// `Date` identifies the instance; `OffsetDateTime` the created-at timestamp.
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime};

// Domain types this module reads/writes.
use amity_core::event_override::{EventOverride, OverrideAction};
use amity_core::ids::{EventId, EventOverrideId, MemberId};

// `StorageError` wraps sqlx and parse failures.
use crate::StorageError;

// The date format for the `instance_date` column (YYYY-MM-DD). It is a
// compile-time-constant bracketed pattern (the `time` crate's format syntax,
// not strftime), parsed once per call in the two helpers below. Shared here so
// the read and write paths cannot drift apart.
const DATE_PATTERN: &str = "[year]-[month]-[day]";

// ─── Row type ────────────────────────────────────────────────────────────────

/// Raw database row from `event_overrides`.
///
/// Everything is TEXT (UUIDs, the date, the action enum, the timestamp) or a
/// nullable TEXT payload; `row_to_override` decodes it. Field names match the
/// SQL columns for `sqlx::FromRow`.
#[derive(sqlx::FromRow)]
struct EventOverrideRow {
    // UUID TEXT — overlay id.
    id: String,
    // UUID TEXT — the overridden event.
    source_event_id: String,
    // YYYY-MM-DD TEXT — the targeted instance date.
    instance_date: String,
    // 'cancel' | 'reschedule' | 'annotate'.
    action: String,
    // Optional action payload: a new RFC 3339 time (reschedule), a note
    // (annotate), or NULL (cancel).
    payload: Option<String>,
    // UUID TEXT — the member who created the overlay.
    created_by: String,
    // RFC 3339 TEXT — created-at timestamp.
    created_at: String,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new event override.
///
/// Overlays are additive, append-only records — there is no update or delete
/// path here (a change of mind is a new overlay). All fields come from the `EventOverride` domain type; the id and timestamp
/// are already set by the caller (`EventOverride::new`), so this function
/// generates nothing and only serialises what it is given.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure (e.g. an FK violation if the
/// event or member does not exist), or `StorageError::Parse` if a value cannot
/// be formatted.
pub async fn insert_event_override(
    pool: &SqlitePool,
    overlay: &EventOverride,
) -> Result<(), StorageError> {
    // Serialise every field to its storage form.
    // Overlay id and the overridden event id, as hyphenated UUID strings.
    let id = overlay.id.to_string();
    let source_event_id = overlay.source_event_id.to_string();
    // The targeted instance date as YYYY-MM-DD TEXT.
    let instance_date = format_date(overlay.instance_date)?;
    // The action enum → 'cancel' | 'reschedule' | 'annotate'.
    let action = overlay.action.to_string();
    // The member who made the change.
    let created_by = overlay.created_by.to_string();
    // The created-at timestamp as RFC 3339 TEXT.
    let created_at = overlay
        .created_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;

    // One row per overlay; there is no upsert — overlays are additive records.
    sqlx::query(
        "
        INSERT INTO event_overrides
            (id, source_event_id, instance_date, action, payload, created_by, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
    )
    // ?1..?7 map to the seven columns in the order declared above.
    .bind(id)
    .bind(source_event_id)
    .bind(instance_date)
    .bind(action)
    // Payload is nullable — None (e.g. for a Cancel) binds as SQL NULL.
    .bind(&overlay.payload)
    .bind(created_by)
    .bind(created_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// List every override targeting the given calendar `date`.
///
/// The surfacing layer calls this once per day and indexes the result by
/// `source_event_id` to decide, per event instance, whether an overlay applies
/// — one query per day rather than one probe per instance.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse` if
/// a stored override row cannot be decoded.
pub async fn list_overrides_on_date(
    pool: &SqlitePool,
    date: Date,
) -> Result<Vec<EventOverride>, StorageError> {
    // A single indexed lookup on instance_date returns the day's overlays.
    // Format the requested day to match the stored YYYY-MM-DD TEXT.
    let date_str = format_date(date)?;

    // Every override row whose instance_date matches the requested day.
    let rows: Vec<EventOverrideRow> = sqlx::query_as(
        "
        SELECT id, source_event_id, instance_date, action, payload, created_by, created_at
        FROM   event_overrides
        WHERE  instance_date = ?1
        ",
    )
    // ?1 the requested day.
    .bind(date_str)
    .fetch_all(pool)
    .await?;

    // Parse each row into the domain type; a corrupt row aborts the whole list.
    rows.into_iter().map(row_to_override).collect()
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Convert a raw row into an `EventOverride`.
///
/// Strict parsing: three UUIDs, the action enum, the instance date, and the
/// timestamp are each decoded, and the first failure aborts the row.
fn row_to_override(row: EventOverrideRow) -> Result<EventOverride, StorageError> {
    // Three UUID columns first.
    // The overlay's own id.
    let id = row
        .id
        .parse::<EventOverrideId>()
        .map_err(|e| StorageError::Parse(format!("override id: {e}")))?;
    // The event this overlay targets.
    let source_event_id = row
        .source_event_id
        .parse::<EventId>()
        .map_err(|e| StorageError::Parse(format!("source_event_id: {e}")))?;
    // The member who created it.
    let created_by = row
        .created_by
        .parse::<MemberId>()
        .map_err(|e| StorageError::Parse(format!("created_by: {e}")))?;

    // The action string back into the enum.
    let action = row
        .action
        .parse::<OverrideAction>()
        .map_err(|e| StorageError::Parse(format!("action: {e}")))?;
    // The targeted calendar date.
    let instance_date = parse_date(&row.instance_date)?;
    // The created-at instant (RFC 3339).
    let created_at = OffsetDateTime::parse(&row.created_at, &Rfc3339)
        .map_err(|e| StorageError::Parse(format!("created_at: {e}")))?;

    // Reassemble the domain type from the parsed parts.
    Ok(EventOverride {
        // Parsed overlay id.
        id,
        // Parsed target-event id.
        source_event_id,
        // Parsed instance date.
        instance_date,
        // Parsed action enum.
        action,
        // Payload passes through as-is (Option).
        payload: row.payload,
        // Parsed member id.
        created_by,
        // Parsed timestamp.
        created_at,
    })
}

/// Format a `Date` as YYYY-MM-DD for storage.
///
/// The same pattern parses the column back in `parse_date`, so writes and reads
/// agree on the format.
fn format_date(date: Date) -> Result<String, StorageError> {
    // The pattern is a compile-time constant, so parsing it never fails.
    let fmt = time::format_description::parse(DATE_PATTERN)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    // Render the date with that description.
    date.format(&fmt)
        .map_err(|e| StorageError::Parse(e.to_string()))
}

/// Parse a YYYY-MM-DD string into a `Date`.
///
/// Names the `instance_date` column in the error so a bad stored value is easy
/// to trace.
fn parse_date(s: &str) -> Result<Date, StorageError> {
    // Same constant pattern as `format_date`, so reads and writes agree.
    let fmt = time::format_description::parse(DATE_PATTERN)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    Date::parse(s, &fmt).map_err(|e| StorageError::Parse(format!("instance_date: {e}")))
}
