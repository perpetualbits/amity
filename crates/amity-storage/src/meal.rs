// meal.rs — repository functions for the Meal entity.
//
// The storage layer's interface for planned meals. It exposes:
//   • insert_meal          — write a new meal + its ingredient lines
//   • fetch_meal            — read one meal (with ingredient lines) by id
//   • list_meals            — read all meals
//   • list_meals_in_range   — read meals whose date falls in [from, to]
//                             (inclusive) — the grocery generator's input
//   • delete_meal           — remove a meal and its ingredient lines
//
// There is no `update_meal` in this slice: the hub's plan-a-meal flow only
// needs create/read/delete for now (per the P2 Slice 2 brief), and an
// update would need to reconcile the `meal_ingredients` child rows the same
// way `delete_meal` + `insert_meal` already can — so editing is deferred to
// a follow-up rather than half-built here.
//
// No business logic lives here; the repository reads and writes the `Meal`
// domain type. See migration 0005.
//
// Storage layout notes:
//   • `Meal.ingredient_lines` is order-sensitive (a `Vec`), but SQL rows have
//     no inherent order, so `meal_ingredients` carries an explicit `position`
//     column written 0..n at insert time and read back via `ORDER BY position`.
//   • `meal_ingredients` rows have their own `id` (required by the STRICT
//     table's PRIMARY KEY) but that id is never surfaced on `IngredientLine`
//     — it exists purely as a row identity, generated fresh on each insert.
//   • `SQLite` STRICT tables have no implicit cascade, so `insert_meal` and
//     `delete_meal` both run in a transaction that touches `meal_ingredients`
//     before `meals`, mirroring `calendar::delete_calendar`.
//   • `Meal.date` is a plain calendar `Date` (no time-of-day, no timezone),
//     stored as `YYYY-MM-DD` TEXT via the `DATE_FORMAT` descriptor — distinct
//     from the RFC 3339 TEXT used for `created_at`. `YYYY-MM-DD` sorts
//     lexicographically the same as calendar order, so `idx_meals_date` and
//     `list_meals_in_range`'s `BETWEEN` both work without a parse step.
//   • Booleans don't appear on this entity; `cook_id` is a nullable TEXT FK
//     to a `MemberId`, flattened directly (no separate columns needed).

// `SqlitePool` is the shared pool injected into every query function.
use sqlx::SqlitePool;
// `Date` is `Meal.date`'s type; `OffsetDateTime` is `created_at`'s.
use time::{Date, OffsetDateTime};
// The RFC 3339 format descriptor for `created_at`.
use time::format_description::well_known::Rfc3339;
// The `YYYY-MM-DD` descriptor for `date`, built once at compile time.
use time::macros::format_description;

// Domain types this module reads/writes.
use amity_core::ids::{MealId, MemberId};
use amity_core::meal::{IngredientLine, Meal, MealBuilder, MealSlot};

// `StorageError` wraps sqlx errors and field-parse failures.
use crate::StorageError;

// ─── Row types ───────────────────────────────────────────────────────────────

/// Raw database row from the `meals` table.
///
/// Every field is `String`/`Option<String>` because the database stores
/// everything as TEXT — there are no typed date, enum, or UUID columns to
/// lean on. Conversion to the domain type (parsing the UUID, the date, the
/// slot enum, and the cook reference) happens in `row_to_meal`, which also
/// needs a separately-fetched `Vec<IngredientLine>` to complete the `Meal`.
#[derive(sqlx::FromRow)]
struct MealRow {
    // UUID TEXT — the meal's primary key.
    id: String,
    // YYYY-MM-DD TEXT — the meal's calendar date.
    date: String,
    // 'dinner' | 'breakfast' | 'lunch' | 'other'.
    slot: String,
    // The meal's name.
    name: String,
    // Cook's MemberId, or NULL if unassigned.
    cook_id: Option<String>,
    // Free-form notes, or NULL.
    notes: Option<String>,
    // RFC 3339 TEXT creation timestamp.
    created_at: String,
}

/// Raw database row from the `meal_ingredients` table.
///
/// Only `name` and `qty` map onto `IngredientLine`; `id` and `meal_id` exist
/// solely for row identity/FK and `position` only orders the read — none of
/// the three round-trip onto the domain type.
#[derive(sqlx::FromRow)]
struct MealIngredientRow {
    // Ingredient name.
    name: String,
    // Freetext quantity, or NULL.
    qty: Option<String>,
}

// ─── Public repository functions ─────────────────────────────────────────────

/// Insert a new meal and its ingredient lines in one transaction.
///
/// The meal row and its `meal_ingredients` rows (written with `position`
/// 0..n, preserving `Meal.ingredient_lines`' `Vec` order) are written
/// together so a reader never observes a meal with a partial ingredient list.
///
/// # Errors
///
/// Returns `StorageError::Database` on any sqlx failure (e.g. a UUID
/// primary-key violation), or `StorageError::Parse` if the date or creation
/// timestamp cannot be formatted.
pub async fn insert_meal(pool: &SqlitePool, meal: &Meal) -> Result<(), StorageError> {
    // UUID newtype → hyphenated string, kept around for the ingredient
    // insert below (which needs the meal's id as a plain &str).
    let id = meal.id.to_string();
    // Calendar date → YYYY-MM-DD TEXT (see the DATE_FORMAT module doc).
    let date = format_date(meal.date)?;
    // Enum → its snake_case storage string.
    let slot = meal.slot.to_string();
    // Optional cook reference → hyphenated string, or None for SQL NULL.
    let cook_id = meal.cook.map(|c| c.to_string());
    // Required creation timestamp → RFC 3339 TEXT.
    let created_at = format_dt(meal.created_at)?;

    // One transaction: the meal row plus every ingredient line, so a
    // mid-write failure never leaves a meal with only some of its lines.
    let mut tx = pool.begin().await?;

    sqlx::query(
        "
        INSERT INTO meals (id, date, slot, name, cook_id, notes, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
    )
    // Bind in the same order as the ?1..?7 placeholders above.
    // ?1 primary key.
    .bind(&id)
    // ?2 calendar date.
    .bind(date)
    // ?3 slot.
    .bind(slot)
    // ?4 name (already validated non-empty by the domain layer).
    .bind(&meal.name)
    // ?5 cook reference, or SQL NULL.
    .bind(cook_id)
    // ?6 notes, or SQL NULL.
    .bind(&meal.notes)
    // ?7 creation timestamp.
    .bind(created_at)
    .execute(&mut *tx)
    .await?;

    // Write the ingredient lines on the same open transaction, so the two
    // writes commit (or roll back) together.
    insert_ingredient_lines(&mut tx, &id, &meal.ingredient_lines).await?;

    tx.commit().await?;
    Ok(())
}

/// Fetch a single meal (with its ordered ingredient lines) by id.
///
/// Returns `None` when no meal has that id — a non-exceptional "not found"
/// the caller maps to a 404 (or ignores) as it sees fit.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored value cannot be decoded.
pub async fn fetch_meal(pool: &SqlitePool, id: MealId) -> Result<Option<Meal>, StorageError> {
    // The typed id becomes a hyphenated UUID string, reused below as the
    // FK value for the ingredient-lines query.
    let id_str = id.to_string();

    // Append the WHERE clause to the shared column list.
    let sql = format!("{MEAL_SELECT} WHERE id = ?1");
    // `fetch_optional` yields None when no row matches.
    let row: Option<MealRow> = sqlx::query_as(&sql)
        .bind(&id_str)
        .fetch_optional(pool)
        .await?;

    // No meal row → no ingredient query needed either.
    let Some(row) = row else {
        return Ok(None);
    };
    // Second query for this meal's ordered ingredient lines.
    let ingredient_lines = fetch_ingredient_lines(pool, &id_str).await?;
    Ok(Some(row_to_meal(row, ingredient_lines)?))
}

/// List all meals, in no particular guaranteed order beyond insertion.
///
/// Each meal's ingredient lines are fetched with one extra query per meal
/// (household meal counts are small enough that this is simpler than a join
/// plus in-memory regrouping, mirroring the household-scale tradeoff already
/// made in `event::list_events`).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored row cannot be decoded.
pub async fn list_meals(pool: &SqlitePool) -> Result<Vec<Meal>, StorageError> {
    // Fetch every meal row; ingredient lines are attached per-row below.
    let rows: Vec<MealRow> = sqlx::query_as(MEAL_SELECT).fetch_all(pool).await?;
    assemble_meals(pool, rows).await
}

/// List meals whose `date` falls within `[from, to]` (inclusive).
///
/// This is the grocery generator's input: it plans a date range (e.g. "this
/// week") and needs exactly the meals in that window, not the whole table.
/// `YYYY-MM-DD` TEXT sorts lexicographically the same as calendar order, so
/// a plain `BETWEEN` on the stored strings is a correct date-range filter.
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure, or `StorageError::Parse`
/// if a stored row cannot be decoded, or if `from`/`to` cannot be formatted.
pub async fn list_meals_in_range(
    pool: &SqlitePool,
    from: Date,
    to: Date,
) -> Result<Vec<Meal>, StorageError> {
    // Bounds → YYYY-MM-DD TEXT, same format as the stored column.
    let from_str = format_date(from)?;
    let to_str = format_date(to)?;

    // Order by date so the caller gets the range chronologically.
    let sql = format!("{MEAL_SELECT} WHERE date BETWEEN ?1 AND ?2 ORDER BY date ASC");
    let rows: Vec<MealRow> = sqlx::query_as(&sql)
        // ?1 inclusive lower bound.
        .bind(from_str)
        // ?2 inclusive upper bound.
        .bind(to_str)
        .fetch_all(pool)
        .await?;
    assemble_meals(pool, rows).await
}

/// Delete a meal and its ingredient lines.
///
/// `SQLite` STRICT tables have no implicit cascade, so `meal_ingredients`
/// rows are deleted before the `meals` row, in one transaction. Returns
/// whether a meal row matched (`false` for an unknown id).
///
/// # Errors
///
/// Returns `StorageError::Database` on sqlx failure.
pub async fn delete_meal(pool: &SqlitePool, id: MealId) -> Result<bool, StorageError> {
    // The typed id becomes a hyphenated UUID string, used for both deletes.
    let id_str = id.to_string();

    // Both deletes must agree on "this meal is gone", so run them in one
    // transaction.
    let mut tx = pool.begin().await?;

    // Deepest dependency first — ingredient lines have no children of their own.
    sqlx::query("DELETE FROM meal_ingredients WHERE meal_id = ?1")
        .bind(&id_str)
        .execute(&mut *tx)
        .await?;

    // The meal row itself; its rows_affected() is the function's result.
    let result = sqlx::query("DELETE FROM meals WHERE id = ?1")
        .bind(&id_str)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    // rows_affected() is 0 or 1 since id is the primary key.
    Ok(result.rows_affected() > 0)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Column list + FROM for the `meals` table. Single source of truth for the
/// select shape; `fetch_meal` and `list_meals_in_range` append their own
/// WHERE / ORDER BY, so the column list is never duplicated.
const MEAL_SELECT: &str = "
    SELECT id, date, slot, name, cook_id, notes, created_at
    FROM   meals
";

/// `YYYY-MM-DD` — the storage format for `Meal.date`, distinct from the RFC
/// 3339 TEXT used for `created_at`. Built once at compile time.
const DATE_FORMAT: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day]");

/// Insert `lines` as `meal_ingredients` rows for `meal_id`, in order, so
/// `position` 0..n reproduces `Vec` order on read. Runs on the caller's open
/// transaction so it shares atomicity with the parent meal write.
async fn insert_ingredient_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    meal_id: &str,
    lines: &[IngredientLine],
) -> Result<(), StorageError> {
    for (position, line) in lines.iter().enumerate() {
        // Ingredient rows have their own id purely for PRIMARY KEY identity;
        // it is never read back onto IngredientLine.
        let row_id = uuid::Uuid::now_v7().to_string();
        // `position` is a small non-negative index; i64 always fits.
        #[allow(clippy::cast_possible_wrap)]
        let position = position as i64;

        sqlx::query(
            "
            INSERT INTO meal_ingredients (id, meal_id, position, name, qty)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ",
        )
        // ?1 row identity (not surfaced on IngredientLine).
        .bind(row_id)
        // ?2 which meal this line belongs to.
        .bind(meal_id)
        // ?3 0-based order, so ORDER BY position reproduces Vec order.
        .bind(position)
        // ?4 ingredient name.
        .bind(&line.name)
        // ?5 freetext quantity, or SQL NULL.
        .bind(&line.qty)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Fetch a meal's ingredient lines, ordered by `position` so the returned
/// `Vec` reproduces the order they were inserted in.
async fn fetch_ingredient_lines(
    pool: &SqlitePool,
    meal_id: &str,
) -> Result<Vec<IngredientLine>, StorageError> {
    let rows: Vec<MealIngredientRow> = sqlx::query_as(
        "SELECT name, qty FROM meal_ingredients WHERE meal_id = ?1 ORDER BY position ASC",
    )
    .bind(meal_id)
    .fetch_all(pool)
    .await?;

    // The row's `id`/`meal_id`/`position` columns did their job in the
    // query (identity, FK, ordering); only `name`/`qty` carry onto the line.
    Ok(rows
        .into_iter()
        .map(|r| IngredientLine {
            name: r.name,
            qty: r.qty,
        })
        .collect())
}

/// Turn a batch of `MealRow`s into `Meal`s, fetching each one's ingredient
/// lines with a follow-up query. Shared by `list_meals` and
/// `list_meals_in_range` so the two list paths cannot drift on assembly.
async fn assemble_meals(pool: &SqlitePool, rows: Vec<MealRow>) -> Result<Vec<Meal>, StorageError> {
    // Pre-size for the exact row count; one extra query and one row_to_meal
    // call per meal.
    let mut meals = Vec::with_capacity(rows.len());
    for row in rows {
        let ingredient_lines = fetch_ingredient_lines(pool, &row.id).await?;
        meals.push(row_to_meal(row, ingredient_lines)?);
    }
    Ok(meals)
}

/// Convert a raw database row (plus its already-fetched ingredient lines)
/// into a `Meal`.
///
/// Parsing is strict: the first field that fails aborts the whole row,
/// because a partially-constructed meal would be more dangerous than a
/// visible error, and the `Vec`-returning callers stop on the first error
/// rather than silently skipping a corrupt row.
fn row_to_meal(row: MealRow, ingredient_lines: Vec<IngredientLine>) -> Result<Meal, StorageError> {
    // Parse the primary key (UUID TEXT → typed newtype).
    let id = row
        .id
        .parse::<MealId>()
        .map_err(|e| StorageError::Parse(format!("meal id: {e}")))?;
    // Calendar date, distinct format from the RFC 3339 timestamps.
    let date = parse_date(&row.date, "date")?;
    // Slot enum via its FromStr impl.
    let slot = row
        .slot
        .parse::<MealSlot>()
        .map_err(|e| StorageError::Parse(format!("slot: {e}")))?;
    // Optional cook reference.
    let cook = row
        .cook_id
        .map(|s| {
            s.parse::<MemberId>()
                .map_err(|e| StorageError::Parse(format!("cook_id: {e}")))
        })
        .transpose()?;
    // Required creation timestamp.
    let created_at = parse_rfc3339(&row.created_at, "created_at")?;

    // Reconstruct via the builder so the same non-empty-name invariant that
    // held at insert time is re-asserted on read; the builder's `id`/`now`
    // hooks exist exactly for this storage-reconstruction path.
    let mut builder = MealBuilder::new(row.name)
        // Validated required field.
        .date(date)
        // Parsed enum.
        .slot(slot)
        // Caller-supplied id, not a freshly minted one.
        .id(id)
        // created_at is set once and never changes.
        .now(created_at);
    // Cook is optional — only set it when the column was non-NULL.
    if let Some(cook) = cook {
        builder = builder.cook(cook);
    }
    // Notes are optional — same pattern as cook above.
    if let Some(notes) = row.notes {
        builder = builder.notes(notes);
    }
    // Re-add each line in stored (position-ordered) order; the builder is
    // additive, so this reproduces the original Vec order exactly.
    for line in ingredient_lines {
        builder = builder.ingredient(line.name, line.qty);
    }

    // build() re-validates the non-empty-name invariant; it cannot fail here
    // in practice since the row was written by this same builder, but the
    // error path is kept live rather than unwrapped for defence in depth.
    builder
        .build()
        .map_err(|e| StorageError::Parse(format!("meal: {e}")))
}

/// Format a `Date` as `YYYY-MM-DD` TEXT (see `DATE_FORMAT`).
fn format_date(date: Date) -> Result<String, StorageError> {
    // Lexicographic order matches calendar order for this format, so
    // ORDER BY / BETWEEN on the stored TEXT need no parse step either.
    date.format(DATE_FORMAT)
        .map_err(|e| StorageError::Parse(e.to_string()))
}

/// Parse a `YYYY-MM-DD` string into `Date`, naming the column on failure.
fn parse_date(s: &str, column: &str) -> Result<Date, StorageError> {
    // `column` appears in the error so a malformed stored value is easy to
    // trace back to its source.
    Date::parse(s, DATE_FORMAT).map_err(|e| StorageError::Parse(format!("{column}: {e}")))
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
