// grocery.rs — repository functions for the GroceryList and GroceryItem entities.
//
// The storage layer's interface for grocery lists. It exposes:
//   • insert_grocery_list       — write a new list
//   • list_grocery_lists        — read all lists
//   • fetch_grocery_list        — read one list by id
//   • insert_grocery_item       — write a single item
//   • insert_grocery_items      — write a batch of items in one transaction
//                                 (used by `plan_grocery_additions`'s output)
//   • list_grocery_items        — read a list's items
//   • set_grocery_item_checked  — toggle an item's checked flag
//   • delete_grocery_item       — remove an item by id
//
// No business logic lives here; the pure generation logic
// (`amity_core::grocery::plan_grocery_additions`) decides *what* to add —
// this module only persists what it is given. See migration 0005.
//
// Storage layout notes:
//   • `GroceryList` and `GroceryItem` are two domain types with their own
//     tables (`grocery_lists`, `grocery_items`), linked by `list_id` — unlike
//     `calendar.rs`'s two-types-one-row pattern, these are genuinely
//     one-to-many and get one table each.
//   • Booleans (`checked`) are INTEGER 0/1 under STRICT mode, matching every
//     other table in this crate.
//   • `source`/`source_meal_id` are the flattened `GrocerySource` — `Manual`
//     items carry `source_meal_id = NULL`, `FromMeal` items carry the
//     originating `MealId`. There is no FK on `source_meal_id` (a meal may be
//     deleted after its groceries were generated; the item should survive).
//   • `insert_grocery_item` and `insert_grocery_items` share one row-binding
//     helper so a single-item insert and the generator's batch insert cannot
//     drift on how a field is encoded.

// `SqlitePool` is the shared pool injected into every query function.
use sqlx::SqlitePool;
// `OffsetDateTime` is the canonical timestamp type; RFC 3339 is the storage form.
use time::OffsetDateTime;
// The RFC 3339 format descriptor used for both serialisation and deserialisation.
use time::format_description::well_known::Rfc3339;

// Domain types this module reads/writes.
use amity_core::grocery::{
    GroceryItem, GroceryItemBuilder, GroceryList, GroceryListBuilder, GrocerySource,
};
use amity_core::ids::{GroceryItemId, GroceryListId, MealId};

// `StorageError` wraps sqlx errors and field-parse failures.
use crate::StorageError;

// ─── Row types ───────────────────────────────────────────────────────────────
// Every field below is String/Option<String>/i64 — the database stores
// everything as TEXT or INTEGER, so typed values are reconstructed only in
// the row_to_* conversion functions further down this file.

/// Raw database row from the `grocery_lists` table.
#[derive(sqlx::FromRow)]
struct GroceryListRow {
    // UUID TEXT — the list's primary key.
    id: String,
    // Display name.
    name: String,
    // RFC 3339 TEXT creation timestamp.
    created_at: String,
}

// Field names above must match the SQL columns exactly for sqlx::FromRow
// to map them without a #[sqlx(rename = "…")] annotation, same rule as
// every other row type in this crate.

/// Raw database row from the `grocery_items` table.
///
/// Every field is `String`/`Option<String>`/`i64` because the database
/// stores everything as TEXT or INTEGER. Conversion to the domain type
/// (parsing UUIDs, the source enum, and the checked flag) happens in
/// `row_to_grocery_item`.
#[derive(sqlx::FromRow)]
struct GroceryItemRow {
    // UUID TEXT — the item's primary key.
    id: String,
    // Which list this item belongs to.
    list_id: String,
    // Item name.
    name: String,
    // Freetext quantity, or NULL.
    qty: Option<String>,
    // Free-form category, or NULL.
    category: Option<String>,
    // 0/1 checked-off flag.
    checked: i64,
    // 'manual' | 'from_meal'.
    source: String,
    // Originating MealId, or NULL for manual items.
    source_meal_id: Option<String>,
    // RFC 3339 TEXT creation timestamp.
    created_at: String,
}

// ─── Public repository functions: lists ──────────────────────────────────────

/// Insert a new grocery list.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure (e.g. a UUID
/// primary-key violation), or `StorageError::Parse` if the creation
/// timestamp cannot be formatted.
pub async fn insert_grocery_list(
    pool: &SqlitePool,
    list: &GroceryList,
) -> Result<(), StorageError> {
    // UUID newtype → hyphenated string.
    let id = list.id.to_string();
    // Required creation timestamp → RFC 3339 TEXT.
    let created_at = format_dt(list.created_at)?;

    sqlx::query("INSERT INTO grocery_lists (id, name, created_at) VALUES (?1, ?2, ?3)")
        // ?1 primary key.
        .bind(id)
        // ?2 name (already validated non-empty by the domain layer).
        .bind(&list.name)
        // ?3 creation timestamp.
        .bind(created_at)
        .execute(pool)
        .await?;

    Ok(())
}

/// List all grocery lists, in no particular guaranteed order beyond insertion.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored row cannot be decoded.
pub async fn list_grocery_lists(pool: &SqlitePool) -> Result<Vec<GroceryList>, StorageError> {
    let rows: Vec<GroceryListRow> = sqlx::query_as(GROCERY_LIST_SELECT).fetch_all(pool).await?;
    // Parse each row; stop at the first error rather than skipping a corrupt row.
    rows.into_iter().map(row_to_grocery_list).collect()
}

/// Fetch a single grocery list by id.
///
/// Returns `None` when no list has that id.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if the stored row cannot be decoded.
pub async fn fetch_grocery_list(
    pool: &SqlitePool,
    id: GroceryListId,
) -> Result<Option<GroceryList>, StorageError> {
    // The typed id becomes a hyphenated UUID string for the bind parameter.
    let id_str = id.to_string();
    // Append the WHERE clause to the shared column list.
    let sql = format!("{GROCERY_LIST_SELECT} WHERE id = ?1");
    // `fetch_optional` yields None when no row matches.
    let row: Option<GroceryListRow> = sqlx::query_as(&sql)
        .bind(id_str)
        .fetch_optional(pool)
        .await?;

    // Parse only when a row exists.
    match row {
        Some(r) => Ok(Some(row_to_grocery_list(r)?)),
        None => Ok(None),
    }
}

// ─── Public repository functions: items ──────────────────────────────────────

/// Insert a single grocery item.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure, or
/// `StorageError::Parse` if the creation timestamp cannot be formatted.
pub async fn insert_grocery_item(
    pool: &SqlitePool,
    item: &GroceryItem,
) -> Result<(), StorageError> {
    // A single-item write still opens a transaction so it shares the exact
    // same code path (`bind_and_insert_item`) as the batch insert below —
    // no separate non-transactional variant to keep in sync.
    let mut tx = pool.begin().await?;
    bind_and_insert_item(&mut tx, item).await?;
    tx.commit().await?;
    Ok(())
}

/// Insert a batch of grocery items in one transaction.
///
/// This is what `plan_grocery_additions`'s caller uses to persist a whole
/// generation run at once: either every addition lands, or (on failure)
/// none do — a partially-applied generation would leave the list in a state
/// the pure function never actually produced.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure, or
/// `StorageError::Parse` if a creation timestamp cannot be formatted.
pub async fn insert_grocery_items(
    pool: &SqlitePool,
    items: &[GroceryItem],
) -> Result<(), StorageError> {
    // One transaction for the whole batch — see the doc comment above for
    // why a partial write is worse than no write.
    let mut tx = pool.begin().await?;
    for item in items {
        bind_and_insert_item(&mut tx, item).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// List a grocery list's items, in no particular guaranteed order beyond
/// insertion. The service layer sorts/groups (e.g. by category) as needed.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored row cannot be decoded.
pub async fn list_grocery_items(
    pool: &SqlitePool,
    list_id: GroceryListId,
) -> Result<Vec<GroceryItem>, StorageError> {
    // The typed id becomes a hyphenated UUID string for the bind parameter.
    let list_id_str = list_id.to_string();
    // Append the WHERE clause to the shared column list.
    let sql = format!("{GROCERY_ITEM_SELECT} WHERE list_id = ?1");
    let rows: Vec<GroceryItemRow> = sqlx::query_as(&sql)
        .bind(list_id_str)
        .fetch_all(pool)
        .await?;
    // Parse each row; stop at the first error rather than skipping a corrupt row.
    rows.into_iter().map(row_to_grocery_item).collect()
}

/// Set a grocery item's checked-off flag.
///
/// Returns whether a row matched (`false` for an unknown id) so the caller
/// can distinguish "toggled" from "nothing to toggle" without a separate
/// existence check.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn set_grocery_item_checked(
    pool: &SqlitePool,
    id: GroceryItemId,
    checked: bool,
) -> Result<bool, StorageError> {
    let id_str = id.to_string();
    // Bool → INTEGER 0/1 under STRICT mode.
    let checked_int = i64::from(checked);

    let result = sqlx::query("UPDATE grocery_items SET checked = ?1 WHERE id = ?2")
        // ?1 the new checked state.
        .bind(checked_int)
        // ?2 which item.
        .bind(id_str)
        .execute(pool)
        .await?;

    // rows_affected() is 0 or 1 since id is the primary key.
    Ok(result.rows_affected() > 0)
}

/// Delete a grocery item by id.
///
/// Returns whether a row matched (`false` for an unknown id).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn delete_grocery_item(
    pool: &SqlitePool,
    id: GroceryItemId,
) -> Result<bool, StorageError> {
    let id_str = id.to_string();

    // Grocery items have no child rows, so a plain single-table delete
    // (no transaction) is enough — unlike meal/calendar deletes.
    let result = sqlx::query("DELETE FROM grocery_items WHERE id = ?1")
        .bind(id_str)
        .execute(pool)
        .await?;

    // rows_affected() is 0 or 1 since id is the primary key.
    Ok(result.rows_affected() > 0)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Column list + FROM for the `grocery_lists` table. Single source of truth
/// for the select shape; `fetch_grocery_list` appends its own `WHERE`.
const GROCERY_LIST_SELECT: &str = "SELECT id, name, created_at FROM grocery_lists";

/// Column list + FROM for the `grocery_items` table. `list_grocery_items`
/// appends its own `WHERE`, so the column list is never duplicated.
const GROCERY_ITEM_SELECT: &str = "
    SELECT id, list_id, name, qty, category, checked, source, source_meal_id, created_at
    FROM   grocery_items
";

/// Bind and execute one `GroceryItem` INSERT against an open transaction.
/// Shared by `insert_grocery_item` and `insert_grocery_items` so the
/// single-item and batch paths cannot drift on column encoding.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if the creation timestamp cannot be formatted.
async fn bind_and_insert_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    item: &GroceryItem,
) -> Result<(), StorageError> {
    // UUID newtypes → hyphenated strings.
    let id = item.id.to_string();
    let list_id = item.list_id.to_string();
    // Bool → INTEGER 0/1 under STRICT mode.
    let checked = i64::from(item.checked);
    // Enum → its snake_case storage string.
    let source = item.source.to_string();
    // Optional originating meal → hyphenated string, or None for SQL NULL.
    let source_meal_id = item.source_meal_id.map(|m| m.to_string());
    // Required creation timestamp → RFC 3339 TEXT.
    let created_at = format_dt(item.created_at)?;

    sqlx::query(
        "
        INSERT INTO grocery_items (
            id, list_id, name, qty, category, checked, source, source_meal_id, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
        )
        ",
    )
    // Bind in the same order as the ?1..?9 placeholders above.
    // ?1 primary key.
    .bind(id)
    // ?2 which list this item belongs to.
    .bind(list_id)
    // ?3 name (already validated non-empty by the domain layer).
    .bind(&item.name)
    // ?4 freetext quantity, or SQL NULL.
    .bind(&item.qty)
    // ?5 category, or SQL NULL.
    .bind(&item.category)
    // ?6 checked-off flag as 0/1.
    .bind(checked)
    // ?7 source kind ('manual' | 'from_meal').
    .bind(source)
    // ?8 originating meal, or SQL NULL for manual items.
    .bind(source_meal_id)
    // ?9 creation timestamp.
    .bind(created_at)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Convert a raw database row into a `GroceryList`.
fn row_to_grocery_list(row: GroceryListRow) -> Result<GroceryList, StorageError> {
    // Parse the primary key (UUID TEXT → typed newtype).
    let id = row
        .id
        .parse::<GroceryListId>()
        .map_err(|e| StorageError::Parse(format!("grocery list id: {e}")))?;
    // Required creation timestamp.
    let created_at = parse_rfc3339(&row.created_at, "created_at")?;

    // Reconstruct via the builder so the non-empty-name invariant that held
    // at insert time is re-asserted on read.
    GroceryListBuilder::new(row.name)
        .id(id)
        .now(created_at)
        .build()
        .map_err(|e| StorageError::Parse(format!("grocery list: {e}")))
}

/// Convert a raw database row into a `GroceryItem`.
///
/// Parsing is strict: the first field that fails aborts the whole row,
/// since a partially-constructed item would be more dangerous than a
/// visible error.
fn row_to_grocery_item(row: GroceryItemRow) -> Result<GroceryItem, StorageError> {
    // Parse the primary key (UUID TEXT → typed newtype).
    let id = row
        .id
        .parse::<GroceryItemId>()
        .map_err(|e| StorageError::Parse(format!("grocery item id: {e}")))?;
    // Which list this item belongs to.
    let list_id = row
        .list_id
        .parse::<GroceryListId>()
        .map_err(|e| StorageError::Parse(format!("list_id: {e}")))?;
    // Enum via its FromStr impl.
    let source = row
        .source
        .parse::<GrocerySource>()
        .map_err(|e| StorageError::Parse(format!("source: {e}")))?;
    // Optional originating meal (None for manual items).
    let source_meal_id = row
        .source_meal_id
        .map(|s| {
            s.parse::<MealId>()
                .map_err(|e| StorageError::Parse(format!("source_meal_id: {e}")))
        })
        .transpose()?;
    // Required creation timestamp.
    let created_at = parse_rfc3339(&row.created_at, "created_at")?;

    // Reconstruct via the builder so the non-empty-name invariant that held
    // at insert time is re-asserted on read.
    let mut builder = GroceryItemBuilder::new(list_id, row.name)
        // Caller-supplied id, not a freshly minted one.
        .id(id)
        // INTEGER 0/1 back to bool.
        .checked(row.checked != 0)
        // Parsed enum.
        .source(source)
        // created_at is set once and never changes.
        .now(created_at);
    // qty/category/source_meal_id are all optional — only set when the
    // column was non-NULL, mirroring the None default on GroceryItemBuilder.
    if let Some(qty) = row.qty {
        builder = builder.qty(qty);
    }
    if let Some(category) = row.category {
        builder = builder.category(category);
    }
    if let Some(source_meal_id) = source_meal_id {
        builder = builder.source_meal_id(source_meal_id);
    }

    // build() re-validates the non-empty-name invariant; kept live (not
    // unwrapped) for defence in depth against a corrupted stored row.
    builder
        .build()
        .map_err(|e| StorageError::Parse(format!("grocery item: {e}")))
}

/// Format a required `OffsetDateTime` as RFC 3339 TEXT.
fn format_dt(dt: OffsetDateTime) -> Result<String, StorageError> {
    dt.format(&Rfc3339)
        .map_err(|e| StorageError::Parse(e.to_string()))
}

/// Parse an RFC 3339 string into `OffsetDateTime`, naming the column on failure.
fn parse_rfc3339(s: &str, column: &str) -> Result<OffsetDateTime, StorageError> {
    OffsetDateTime::parse(s, &Rfc3339).map_err(|e| StorageError::Parse(format!("{column}: {e}")))
}
