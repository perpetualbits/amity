// inbox.rs — repository functions for InboxItem.
//
// This module is the storage layer's interface for the Inbox entity.
// It exposes three functions matching the service's needs:
//   • insert_inbox_item   — write a new item to the database
//   • fetch_inbox_item    — read one item by ID
//   • list_recent_inbox_items — read the N most recent items
//
// Design constraints (from the task spec and rust guidelines):
//   • No business logic here. The repository writes and reads domain types;
//     it does not validate, transform, or make decisions.
//   • We use runtime `sqlx::query` / `sqlx::query_as` rather than the
//     compile-time `query!` macro because the offline query cache has not been
//     generated yet. A follow-up task will run `cargo sqlx prepare` and switch
//     to the checked macros for schema-mismatch safety at compile time.
//   • UUIDs are stored as TEXT; the conversions happen explicitly in this module.
//   • OffsetDateTime is stored as ISO-8601 TEXT via time's Rfc3339 formatter.

use sqlx::SqlitePool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use amity_core::ids::{InboxItemId, MemberId};
use amity_core::inbox::{InboxItem, InboxSource, TriageState, TypedEntityRef};

use crate::StorageError;

// ─── Row type for sqlx query_as ──────────────────────────────────────────────

/// Raw database row from the `inbox_items` table.
///
/// sqlx maps column names to field names by convention. All fields are `String`
/// or `Option<String>` because we store everything as TEXT; the conversion to
/// domain types happens in `row_to_inbox_item`.
#[derive(sqlx::FromRow)]
struct InboxItemRow {
    // UUID v7 as hyphenated TEXT; matches the InboxItemId::to_string() format.
    id: String,
    // Verbatim captured text — never modified on read.
    raw_text: String,
    // UUID of the capturing member, hyphenated TEXT.
    captured_by: String,
    // RFC 3339 timestamp string, e.g. "2026-05-25T10:00:00Z".
    captured_at: String,
    // snake_case variant name (e.g. "touch", "voice") — see InboxSource::Display.
    source: String,
    // snake_case triage state (e.g. "untriaged", "typed") — see TriageState::Display.
    triage_state: String,
    // NULL when untriaged; "entity_type:uuid" when the item has been typed.
    triaged_to: Option<String>,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new inbox item into the database.
///
/// The item's `id` and all fields are taken from the `InboxItem` struct; this
/// function does not generate IDs or timestamps — that is the service's job.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure (constraint violation,
/// I/O error, pool exhaustion).
pub async fn insert_inbox_item(pool: &SqlitePool, item: &InboxItem) -> Result<(), StorageError> {
    // Convert each domain type to its TEXT representation for the SQL bind.
    let id = item.id.to_string();
    // MemberId implements Display as a hyphenated UUID string.
    let captured_by = item.captured_by.to_string();
    // RFC 3339 is a profile of ISO-8601 that SQLite's text affinity sorts
    // correctly because it uses a fixed-width format with Z or ±HH:MM offset.
    let captured_at = item
        .captured_at
        .format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))?;
    // InboxSource::Display produces the snake_case storage string (see inbox.rs).
    let source = item.source.to_string();
    // TriageState::Display produces the snake_case storage string.
    let triage_state = item.triage_state.to_string();
    // triaged_to is NULL when absent — straightforward Option→NULL mapping.
    let triaged_to = item.triaged_to.as_ref().map(|r| r.0.clone());

    // Positional bind parameters (?1..?7) map to the VALUES list in order.
    // Named parameters are not used here to keep the query portable.
    sqlx::query(
        "
        INSERT INTO inbox_items
            (id, raw_text, captured_by, captured_at, source, triage_state, triaged_to)
        VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
    )
    .bind(id)
    // raw_text is bound as a reference — no copy needed; sqlx borrows the String.
    .bind(&item.raw_text)
    // captured_by, captured_at in bind order.
    .bind(captured_by)
    .bind(captured_at)
    // source and triage_state are snake_case strings from Display.
    .bind(source)
    .bind(triage_state)
    // triaged_to is None→NULL or Some(String)→TEXT in SQLite.
    .bind(triaged_to)
    // `.execute` runs the INSERT without returning rows.
    .execute(pool)
    .await?;

    // Return unit on success. The caller already has the full InboxItem struct.
    Ok(())
}

/// Fetch a single inbox item by its ID.
///
/// Returns `None` if no item with that ID exists in the database.
/// "Not found" is a valid result, not an error — the caller decides what to
/// do with a missing item.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
/// Returns `StorageError::Parse` if a stored value cannot be decoded into its
/// domain type (e.g. an unknown `source` string added by a newer binary).
pub async fn fetch_inbox_item(
    pool: &SqlitePool,
    id: InboxItemId,
) -> Result<Option<InboxItem>, StorageError> {
    // Convert ID to string for the SQL parameter — all IDs are stored as TEXT.
    let id_str = id.to_string();

    // `fetch_optional` returns None when no row matches — correct for "not found".
    let row: Option<InboxItemRow> = sqlx::query_as(
        "
        SELECT id, raw_text, captured_by, captured_at, source, triage_state, triaged_to
        FROM   inbox_items
        WHERE  id = ?1
        ",
    )
    // Bind the UUID string to the WHERE clause parameter.
    .bind(id_str)
    // fetch_optional returns None when no row matches — correct for "not found".
    .fetch_optional(pool)
    .await?;

    // Transpose the Option: if there's no row, return None immediately without
    // calling `row_to_inbox_item` — avoids parsing work for the not-found path.
    match row {
        Some(r) => Ok(Some(row_to_inbox_item(r)?)),
        None => Ok(None),
    }
}

/// List the most recent inbox items, newest first.
///
/// `limit` caps the number of rows returned. Pass `20` for the default view.
/// The API layer enforces a maximum of 100 (see `amity-service::api::inbox`).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
/// Returns `StorageError::Parse` if any stored row contains an unrecognised
/// enum value.
pub async fn list_recent_inbox_items(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<InboxItem>, StorageError> {
    // SQLite LIMIT takes a signed 64-bit integer; i64::from is lossless for u32.
    let limit_i64 = i64::from(limit);

    // `fetch_all` returns all matching rows as a Vec. Lists are capped at 100
    // by the API layer, so memory usage is bounded.
    let rows: Vec<InboxItemRow> = sqlx::query_as(
        "
        SELECT id, raw_text, captured_by, captured_at, source, triage_state, triaged_to
        FROM   inbox_items
        ORDER  BY captured_at DESC
        LIMIT  ?1
        ",
    )
    // Bind the row count limit to the LIMIT clause.
    .bind(limit_i64)
    // fetch_all returns all matching rows into a Vec.
    .fetch_all(pool)
    .await?;

    // `.collect()` on `Iterator<Item=Result<T,E>>` stops at the first error —
    // a parse failure on any row surfaces immediately rather than silently
    // omitting corrupted rows from the result list.
    rows.into_iter().map(row_to_inbox_item).collect()
}

// ─── Private helpers ─────────────────────────────────────────────────────────

/// Convert a raw database row into an `InboxItem`.
///
/// Extracted as a helper so the two query sites (`fetch_inbox_item`,
/// `list_recent_inbox_items`) share the same parsing logic and any fix
/// lands in one place.
fn row_to_inbox_item(row: InboxItemRow) -> Result<InboxItem, StorageError> {
    // All parse steps follow the same pattern: attempt the parse, map the error
    // to StorageError::Parse with the column name in the message for easy diagnosis.
    let id = row
        .id
        .parse::<InboxItemId>()
        .map_err(|e| StorageError::Parse(format!("id: {e}")))?;

    // Parse captured_by: UUID string → MemberId.
    let captured_by = row
        .captured_by
        .parse::<MemberId>()
        .map_err(|e| StorageError::Parse(format!("captured_by: {e}")))?;

    // OffsetDateTime::parse with Rfc3339 requires the string to be a valid
    // RFC 3339 timestamp. All values written by this module use Rfc3339 on
    // the write path, so a parse failure here is a data integrity error.
    let captured_at = OffsetDateTime::parse(&row.captured_at, &Rfc3339)
        .map_err(|e| StorageError::Parse(format!("captured_at: {e}")))?;

    // InboxSource::from_str validates the set of known variant strings;
    // an unknown string means a newer binary wrote a value this binary can't read.
    let source = row
        .source
        .parse::<InboxSource>()
        .map_err(|e| StorageError::Parse(format!("source: {e}")))?;

    // TriageState parsing mirrors InboxSource — same error shape and semantics.
    let triage_state = row
        .triage_state
        .parse::<TriageState>()
        .map_err(|e| StorageError::Parse(format!("triage_state: {e}")))?;

    // NULL triaged_to maps to None; a non-NULL value wraps the string as-is.
    let triaged_to = row.triaged_to.map(TypedEntityRef);

    // Construct the domain type. All fields have been validated above.
    Ok(InboxItem {
        id,
        // raw_text is stored verbatim; no transformation on read.
        raw_text: row.raw_text,
        // captured_by holds the capturing member's UUID-based identifier.
        captured_by,
        // captured_at was validated as RFC 3339 and carries the UTC offset.
        captured_at,
        // source is the mechanism through which this item was captured.
        source,
        // triage_state is the current lifecycle stage of this item.
        triage_state,
        // triaged_to is None for untriaged items; Some for typed entities.
        triaged_to,
    })
}
