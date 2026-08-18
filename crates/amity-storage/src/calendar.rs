// calendar.rs — repository functions for the Calendar entity.
//
// The storage layer's interface for subscribed external ICS calendars. It
// exposes:
//   • insert_calendar             — write a new subscription (fresh sync state)
//   • list_calendars              — read all calendars, with their sync state
//   • fetch_calendar              — read one calendar (with sync state) by id
//   • set_calendar_enabled        — toggle whether the sync job fetches a feed
//   • update_calendar_sync_state  — record the outcome of a sync attempt
//   • delete_calendar             — remove a subscription and its events
//
// No business logic lives here; the repository reads and writes the
// `Calendar`/`CalendarSyncState` domain types. See migration 0004.
//
// Storage layout notes:
//   • `Calendar` and `CalendarSyncState` are two domain types (a stable
//     description vs. mutable runtime health, see amity_core::calendar) but
//     share ONE `calendars` row — there is no separate sync-state table. The
//     read model `StoredCalendar` bundles both back together for callers.
//   • Booleans (`enabled`) are INTEGER 0/1 under STRICT mode, matching every
//     other table in this crate.
//   • Datetimes are RFC 3339 TEXT with an explicit offset, so `last_synced_at`
//     sorts and compares correctly without a parse step.
//   • `delete_calendar` cascades manually: SQLite's STRICT tables have no
//     implicit `ON DELETE CASCADE` unless declared on the foreign key, and
//     migration 0003's `event_instances.event_id` reference does not declare
//     one, so the dependent rows are deleted in dependency order (instances,
//     then events, then the calendar) inside this function.

// `SqlitePool` is the shared pool injected into every query function.
use sqlx::SqlitePool;
// `OffsetDateTime` is the canonical timestamp type; RFC 3339 is the storage form.
use time::OffsetDateTime;
// The RFC 3339 format descriptor used for both serialisation and deserialisation.
use time::format_description::well_known::Rfc3339;

// Domain types this module reads/writes.
use amity_core::calendar::{Calendar, CalendarCategory, CalendarSyncState, SyncStatus};
use amity_core::ids::CalendarId;

// `StorageError` wraps sqlx errors and field-parse failures.
use crate::StorageError;

// ─── Read model ────────────────────────────────────────────────────────────────

/// A calendar together with its current sync health.
///
/// The two domain types share one `calendars` row (see module docs); this is
/// the shape every read path (`list_calendars`, `fetch_calendar`) returns so
/// callers never have to join them back together themselves.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCalendar {
    /// The subscription itself: name, URL, category, enabled flag.
    pub calendar: Calendar,
    /// The most recent sync attempt's outcome.
    pub sync: CalendarSyncState,
}

// ─── Row type ────────────────────────────────────────────────────────────────

/// Raw database row from the `calendars` table.
///
/// Every field is `String`/`Option<String>`/`i64` because the database stores
/// everything as TEXT or INTEGER. Conversion to the domain types (parsing the
/// UUID, the enums, the datetimes, and the counters) happens in
/// `row_to_stored`. Field names must match the SQL columns exactly for
/// `sqlx::FromRow` to map them without a `#[sqlx(rename = "…")]` annotation.
#[derive(sqlx::FromRow)]
struct CalendarRow {
    // UUID TEXT — the calendar's primary key.
    id: String,
    // Display name.
    name: String,
    // Feed URL (already normalised to http(s) by the domain layer).
    url: String,
    // 'school' | 'club' | 'waste' | 'holiday' | 'personal' | 'other'.
    category: String,
    // 0/1 enabled flag.
    enabled: i64,
    // RFC 3339 TEXT creation timestamp.
    created_at: String,
    // RFC 3339 TEXT last-success time, or NULL.
    last_synced_at: Option<String>,
    // 'never' | 'ok' | 'unreachable' | 'parse_error'.
    last_status: String,
    // Short diagnostic on failure, or NULL.
    last_error: Option<String>,
    // Event count from the last good sync.
    event_count: i64,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new calendar subscription with fresh (never-synced) sync state.
///
/// The sync-state columns are set to their `CalendarSyncState::default()`
/// equivalents (`last_status = "never"`, `event_count = 0`,
/// `last_synced_at = NULL`) — a freshly subscribed feed has not run yet, so
/// the caller never supplies sync state at insert time.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure (e.g. a primary-key
/// violation), or `StorageError::Parse` if a timestamp cannot be formatted.
pub async fn insert_calendar(pool: &SqlitePool, calendar: &Calendar) -> Result<(), StorageError> {
    // Serialise every field to its TEXT/INTEGER storage form up front, so the
    // bind chain below stays a straight list.
    let id = calendar.id.to_string();
    let category = calendar.category.to_string();
    let enabled = i64::from(calendar.enabled);
    let created_at = format_dt(calendar.created_at)?;
    // A freshly subscribed feed has never synced — the default sync state.
    let last_status = SyncStatus::Never.to_string();

    sqlx::query(
        "
        INSERT INTO calendars (
            id, name, url, category, enabled, created_at,
            last_synced_at, last_status, last_error, event_count
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            NULL, ?7, NULL, 0
        )
        ",
    )
    // ?1 primary key.
    .bind(id)
    // ?2 display name.
    .bind(&calendar.name)
    // ?3 feed URL.
    .bind(&calendar.url)
    // ?4 category.
    .bind(category)
    // ?5 enabled flag as 0/1.
    .bind(enabled)
    // ?6 creation timestamp.
    .bind(created_at)
    // ?7 last_status, always "never" on insert; last_synced_at/last_error/
    // event_count are hard-coded NULL/NULL/0 in the VALUES list above.
    .bind(last_status)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all calendars, in no particular guaranteed order beyond insertion.
///
/// At household scale the calendar count is small (a handful of feeds), so a
/// full unordered scan is simplest; callers that need a display order sort in
/// the service layer.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored row cannot be decoded.
pub async fn list_calendars(pool: &SqlitePool) -> Result<Vec<StoredCalendar>, StorageError> {
    let rows: Vec<CalendarRow> = sqlx::query_as(CALENDAR_SELECT).fetch_all(pool).await?;

    // Parse each row; stop at the first error rather than skipping a corrupt row.
    rows.into_iter().map(row_to_stored).collect()
}

/// Fetch a single calendar (with its sync state) by id.
///
/// Returns `None` when no calendar has that id — a non-exceptional "not
/// found" the caller maps to a 404 (or ignores) as it sees fit.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if the stored row cannot be decoded.
pub async fn fetch_calendar(
    pool: &SqlitePool,
    id: CalendarId,
) -> Result<Option<StoredCalendar>, StorageError> {
    // The typed id becomes a hyphenated UUID string for the bind parameter.
    let id_str = id.to_string();

    // Append the WHERE clause to the shared column list via format!, since
    // concat! requires a literal and CALENDAR_SELECT is a const &str.
    let sql = format!("{CALENDAR_SELECT} WHERE id = ?1");
    let row: Option<CalendarRow> = sqlx::query_as(&sql)
        .bind(id_str)
        .fetch_optional(pool)
        .await?;

    match row {
        Some(r) => Ok(Some(row_to_stored(r)?)),
        None => Ok(None),
    }
}

/// Enable or disable a calendar's syncing without touching its sync state.
///
/// Returns whether a row matched (`false` for an unknown id) so the caller
/// can distinguish "toggled" from "nothing to toggle" without a separate
/// existence check.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn set_calendar_enabled(
    pool: &SqlitePool,
    id: CalendarId,
    enabled: bool,
) -> Result<bool, StorageError> {
    let id_str = id.to_string();
    let enabled_int = i64::from(enabled);

    let result = sqlx::query("UPDATE calendars SET enabled = ?1 WHERE id = ?2")
        .bind(enabled_int)
        .bind(id_str)
        .execute(pool)
        .await?;

    // rows_affected() is 0 or 1 since id is the primary key.
    Ok(result.rows_affected() > 0)
}

/// Delete a calendar subscription and every event/instance it owns.
///
/// `SQLite` STRICT tables have no implicit cascade, so the dependent rows are
/// removed in dependency order: `event_instances` for this calendar's events,
/// then the `events` themselves, then the `calendars` row. Returns whether a
/// calendar row matched (`false` for an unknown id).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn delete_calendar(pool: &SqlitePool, id: CalendarId) -> Result<bool, StorageError> {
    let id_str = id.to_string();

    // A transaction keeps the three-table cascade atomic — either all rows
    // for this calendar disappear together, or none do.
    let mut tx = pool.begin().await?;

    // Step 1: instances of this calendar's events (deepest dependency first).
    sqlx::query(
        "
        DELETE FROM event_instances
        WHERE event_id IN (SELECT id FROM events WHERE source_calendar_id = ?1)
        ",
    )
    .bind(&id_str)
    .execute(&mut *tx)
    .await?;

    // Step 2: the events themselves.
    sqlx::query("DELETE FROM events WHERE source_calendar_id = ?1")
        .bind(&id_str)
        .execute(&mut *tx)
        .await?;

    // Step 3: the calendar row; its rows_affected() is the function's result.
    let result = sqlx::query("DELETE FROM calendars WHERE id = ?1")
        .bind(&id_str)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(result.rows_affected() > 0)
}

/// Record the outcome of a sync attempt for a calendar.
///
/// Overwrites all four sync-state columns from the supplied
/// `CalendarSyncState` in one statement — the sync job always has the full
/// state (it just computed it), so there is no partial-update variant.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if `last_synced_at` cannot be formatted.
pub async fn update_calendar_sync_state(
    pool: &SqlitePool,
    id: CalendarId,
    state: &CalendarSyncState,
) -> Result<(), StorageError> {
    let id_str = id.to_string();
    let last_synced_at = format_optional_dt(state.last_synced_at)?;
    let last_status = state.last_status.to_string();
    // event_count is a u32 in the domain type; SQLite INTEGER binds as i64.
    let event_count = i64::from(state.event_count);

    sqlx::query(
        "
        UPDATE calendars
        SET    last_synced_at = ?1, last_status = ?2, last_error = ?3, event_count = ?4
        WHERE  id = ?5
        ",
    )
    // ?1 last success time, or NULL.
    .bind(last_synced_at)
    // ?2 status string.
    .bind(last_status)
    // ?3 diagnostic, or NULL.
    .bind(&state.last_error)
    // ?4 event count from the last good sync.
    .bind(event_count)
    // ?5 which calendar.
    .bind(id_str)
    .execute(pool)
    .await?;

    Ok(())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Column list + FROM for the `calendars` table. Single source of truth for
/// the select shape; `fetch_calendar` appends `WHERE id = ?1` via `format!`
/// (never `concat!`, which needs a compile-time literal), so the column list
/// is never duplicated.
const CALENDAR_SELECT: &str = "
    SELECT id, name, url, category, enabled, created_at,
           last_synced_at, last_status, last_error, event_count
    FROM   calendars
";

/// Convert a raw database row into a `StoredCalendar`.
///
/// Parsing is strict: the first field that fails aborts the whole row, since
/// a partially-constructed calendar would be more dangerous than a visible
/// error, and the `Vec`-returning callers stop on the first error rather than
/// silently skipping a corrupt row.
fn row_to_stored(row: CalendarRow) -> Result<StoredCalendar, StorageError> {
    // Parse the primary key (UUID TEXT → typed newtype).
    let id = row
        .id
        .parse::<CalendarId>()
        .map_err(|e| StorageError::Parse(format!("calendar id: {e}")))?;
    // Parse the two stored enums via their FromStr impls.
    let category = row
        .category
        .parse::<CalendarCategory>()
        .map_err(|e| StorageError::Parse(format!("category: {e}")))?;
    let last_status = row
        .last_status
        .parse::<SyncStatus>()
        .map_err(|e| StorageError::Parse(format!("last_status: {e}")))?;
    // Required creation timestamp.
    let created_at = parse_rfc3339(&row.created_at, "created_at")?;
    // Optional last-sync timestamp (None when the column is NULL).
    let last_synced_at = row
        .last_synced_at
        .map(|s| parse_rfc3339(&s, "last_synced_at"))
        .transpose()?;

    let calendar = Calendar {
        id,
        name: row.name,
        url: row.url,
        category,
        // INTEGER 0/1 back to bool.
        enabled: row.enabled != 0,
        created_at,
    };
    let sync = CalendarSyncState {
        last_synced_at,
        last_status,
        last_error: row.last_error,
        // Stored as INTEGER (i64); the domain type is u32. A negative or
        // over-large stored value indicates a storage-level bug, not user
        // input, so this narrows with a lossy cast rather than propagating a
        // new error variant — event counts never approach u32::MAX.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        event_count: row.event_count as u32,
    };

    Ok(StoredCalendar { calendar, sync })
}

/// Format a required `OffsetDateTime` as RFC 3339 TEXT.
fn format_dt(dt: OffsetDateTime) -> Result<String, StorageError> {
    dt.format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))
}

/// Format an optional `OffsetDateTime` as RFC 3339, or `None` for SQL NULL.
fn format_optional_dt(dt: Option<OffsetDateTime>) -> Result<Option<String>, StorageError> {
    dt.map(|t| {
        t.format(&Rfc3339)
            .map_err(|e| StorageError::Parse(e.to_string()))
    })
    .transpose()
}

/// Parse an RFC 3339 string into `OffsetDateTime`, naming the column on failure.
fn parse_rfc3339(s: &str, column: &str) -> Result<OffsetDateTime, StorageError> {
    OffsetDateTime::parse(s, &Rfc3339).map_err(|e| StorageError::Parse(format!("{column}: {e}")))
}
