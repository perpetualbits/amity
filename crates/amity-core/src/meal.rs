// meal.rs — the Meal domain type.
//
// Meal is P2 Slice 1's planning entity: a single meal on a single date, with
// freetext ingredient lines that later feed the grocery-generation function
// in `grocery.rs`. This phase is deliberately narrow — no recipes, no
// nutrition, no cook-assignment logic beyond a plain optional reference.
// See the P2 Slice 1 brief for the fixed scope of this phase.
//
// This module defines:
//   • `MealSlot`        — enum: Dinner (default) | Breakfast | Lunch | Other.
//   • `IngredientLine`   — a freetext ingredient: name + optional freetext qty.
//   • `Meal`             — the full domain struct (fields pub for storage
//                           reconstruction, mirroring `Task`/`Event`).
//   • `MealBuilder`      — builder that validates invariants before construction.
//   • `MealError`        — error variants for builder and parse failures.
//
// Ingredient quantities are intentionally freetext ("2 lb", "a bunch") — this
// phase does not parse units or do arithmetic. Aggregation across meals for
// grocery generation is by ingredient *name* only (see `grocery.rs`).

// Serde derives for JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// `Date` for the meal's calendar date; `OffsetDateTime` for `created_at`.
use time::{Date, OffsetDateTime};

// Typed IDs for this entity and the member it optionally references.
use crate::ids::{MealId, MemberId};

// ─── MealSlot ─────────────────────────────────────────────────────────────────

/// Which slot of the day a `Meal` occupies.
///
/// `Dinner` is the default — it is overwhelmingly the slot households plan
/// ahead of time. The other variants exist so the same entity covers the
/// occasional planned breakfast or lunch without a separate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MealSlot {
    /// The evening meal — the default slot for planned meals.
    #[default]
    Dinner,
    /// The morning meal.
    Breakfast,
    /// The midday meal.
    Lunch,
    /// Anything that doesn't fit the standard three (a snack spread, a
    /// potluck contribution, etc).
    Other,
}

impl std::fmt::Display for MealSlot {
    /// Produce the `snake_case` storage string.
    ///
    /// These strings are the database storage contract. Changing them requires
    /// a migration to backfill existing rows.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match keeps this independent of serde for SQL bind parameters.
        let s = match self {
            Self::Dinner => "dinner",
            Self::Breakfast => "breakfast",
            Self::Lunch => "lunch",
            Self::Other => "other",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for MealSlot {
    type Err = MealError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`MealError::UnknownSlot`] if the string is not a known variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm mirrors the Display impl above for round-trip safety.
        match s {
            "dinner" => Ok(Self::Dinner),
            "breakfast" => Ok(Self::Breakfast),
            "lunch" => Ok(Self::Lunch),
            "other" => Ok(Self::Other),
            other => Err(MealError::UnknownSlot(other.to_owned())),
        }
    }
}

// ─── IngredientLine ─────────────────────────────────────────────────────────

/// One ingredient needed for a `Meal`.
///
/// `qty` is deliberately freetext ("2 lb", "a bunch", "1") — this phase does
/// not parse units or do arithmetic across lines. Aggregation for grocery
/// generation happens by `name` only (see `grocery::plan_grocery_additions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngredientLine {
    /// The ingredient's name, as typed by the household (e.g. "flour").
    pub name: String,
    /// Freetext quantity, if the household chose to record one.
    pub qty: Option<String>,
}

// ─── Meal ───────────────────────────────────────────────────────────────────

/// A single planned meal on a single date.
///
/// Construction goes through [`MealBuilder`]. Direct field construction is
/// `pub` so the storage layer can reconstruct meals from database rows —
/// callers outside the storage layer should use the builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meal {
    /// Globally unique, time-ordered identifier.
    pub id: MealId,
    /// The calendar date this meal is planned for.
    pub date: Date,
    /// Which slot of the day (dinner by default).
    pub slot: MealSlot,
    /// The meal's name (e.g. "Spaghetti bolognese"). Must not be empty after
    /// trimming.
    pub name: String,
    /// The member who is cooking, if assigned. A plain reference — no
    /// cook-assignment logic (rotation, fairness, etc) lives in this phase.
    pub cook: Option<MemberId>,
    /// Freetext ingredient lines for this meal. May be empty.
    pub ingredient_lines: Vec<IngredientLine>,
    /// Optional free-form notes (e.g. "double the sauce for leftovers").
    pub notes: Option<String>,
    /// When this meal was first created.
    pub created_at: OffsetDateTime,
}

// ─── MealBuilder ────────────────────────────────────────────────────────────

/// Builder for [`Meal`].
///
/// Enforces the invariants the struct requires:
/// - `name` must not be empty after trimming.
/// - `date` must be set.
/// - `now` must be set (used for `created_at`).
///
/// Optional fields default sensibly: `slot` to `Dinner`, `cook` to `None`,
/// `ingredient_lines` to an empty vec, `notes` to `None`.
#[derive(Debug)]
pub struct MealBuilder {
    /// The meal name. Required; validated on `build()`.
    name: Option<String>,
    /// Calendar date. Required.
    date: Option<Date>,
    /// Slot of the day; defaults to Dinner.
    slot: MealSlot,
    /// Cook reference, if assigned.
    cook: Option<MemberId>,
    /// Accumulated ingredient lines.
    ingredient_lines: Vec<IngredientLine>,
    /// Optional free-form notes.
    notes: Option<String>,
    /// Caller-supplied id, used by the storage layer to reconstruct a row
    /// with its existing id rather than minting a fresh one.
    id: Option<MealId>,
    /// Current wall-clock time for `created_at`. Required.
    now: Option<OffsetDateTime>,
}

impl MealBuilder {
    /// Start building a new meal with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            // Store as-is; the non-empty check happens in build().
            name: Some(name.into()),
            // date is required; None triggers MissingDate in build().
            date: None,
            // Dinner is the overwhelming default use case.
            slot: MealSlot::Dinner,
            // No cook assigned unless the caller sets one.
            cook: None,
            // No ingredients unless the caller adds them.
            ingredient_lines: Vec::new(),
            // No notes unless set.
            notes: None,
            // No caller-supplied id unless the storage layer sets one.
            id: None,
            // now must be supplied for deterministic timestamps.
            now: None,
        }
    }

    /// Set the meal's calendar date (required).
    #[must_use]
    pub fn date(mut self, date: Date) -> Self {
        // Wrap in Some; absence triggers MissingDate in build().
        self.date = Some(date);
        self
    }

    /// Set the slot of the day (defaults to `Dinner`).
    #[must_use]
    pub fn slot(mut self, slot: MealSlot) -> Self {
        // Overrides the Dinner default — used for the occasional planned
        // breakfast or lunch.
        self.slot = slot;
        self
    }

    /// Set the member who is cooking.
    #[must_use]
    pub fn cook(mut self, cook: MemberId) -> Self {
        // Wrap in Some; None means "not yet assigned".
        self.cook = Some(cook);
        self
    }

    /// Add one ingredient line (name + optional freetext quantity).
    ///
    /// Multiple calls accumulate lines — the list is additive.
    #[must_use]
    pub fn ingredient(mut self, name: impl Into<String>, qty: Option<String>) -> Self {
        // Push a new line; qty is stored verbatim (freetext, no parsing).
        self.ingredient_lines.push(IngredientLine {
            name: name.into(),
            qty,
        });
        self
    }

    /// Set optional free-form notes.
    #[must_use]
    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        // Wrap in Some to signal that notes were provided.
        self.notes = Some(notes.into());
        self
    }

    /// Supply a specific id (used by the storage layer to reconstruct a row
    /// with its existing id). Production callers building a brand-new meal
    /// should not call this — `build()` mints a fresh id when absent.
    #[must_use]
    pub fn id(mut self, id: MealId) -> Self {
        // Overrides the fresh-id-on-build default; storage reconstruction is
        // the only caller that should ever need this.
        self.id = Some(id);
        self
    }

    /// Supply the current wall-clock time for `created_at` (required).
    ///
    /// Production callers pass `OffsetDateTime::now_utc()`; tests pass a
    /// fixed value for determinism.
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        // Wrap in Some; absence triggers MissingNow in build().
        self.now = Some(now);
        self
    }

    /// Validate all invariants and construct the `Meal`.
    ///
    /// # Errors
    ///
    /// Returns [`MealError::EmptyName`] if the name is blank.
    /// Returns [`MealError::MissingDate`] if `date` was not set.
    /// Returns [`MealError::MissingNow`] if `now` was not set.
    pub fn build(self) -> Result<Meal, MealError> {
        // Name is set unconditionally in new(); validate its trimmed form.
        let name = self.name.unwrap_or_default();
        if name.trim().is_empty() {
            return Err(MealError::EmptyName);
        }

        // date and now are both required for a complete Meal.
        let date = self.date.ok_or(MealError::MissingDate)?;
        let now = self.now.ok_or(MealError::MissingNow)?;

        // Use the caller-supplied id (storage reconstruction) or mint a
        // fresh time-ordered one (new-meal construction).
        let id = self.id.unwrap_or_default();

        Ok(Meal {
            // Freshly generated or caller-supplied above.
            id,
            // Validated required field.
            date,
            // Dinner unless the caller overrode it.
            slot: self.slot,
            // Validated non-empty; stored verbatim.
            name,
            // None unless a cook was assigned.
            cook: self.cook,
            // Empty unless the caller added lines via .ingredient().
            ingredient_lines: self.ingredient_lines,
            // None unless the caller set notes.
            notes: self.notes,
            // created_at is set once and never changes.
            created_at: now,
        })
    }
}

// ─── MealError ──────────────────────────────────────────────────────────────

/// Errors produced when building or parsing `Meal`-related types.
#[derive(Debug, thiserror::Error)]
pub enum MealError {
    /// `name` was absent or contained only whitespace.
    #[error("meal name must not be empty")]
    EmptyName,

    /// `date` was not set on the builder.
    #[error("meal date is required")]
    MissingDate,

    /// `now` was not set on the builder.
    #[error("required field not set: now")]
    MissingNow,

    /// The database contained an unrecognised `slot` string.
    #[error("unknown meal slot: {0}")]
    UnknownSlot(String),
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    /// Fixed "now" for deterministic tests.
    fn fixed_now() -> OffsetDateTime {
        datetime!(2026-08-22 10:00:00 UTC)
    }

    /// Fixed date for deterministic tests.
    fn fixed_date() -> Date {
        date!(2026 - 08 - 24)
    }

    /// Placeholder member id (matches migration 0001 value).
    fn placeholder_member() -> MemberId {
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    #[test]
    fn build_minimal_meal() {
        // Happy path: only the required fields (name, date, now) are set.
        let meal = MealBuilder::new("Spaghetti")
            .date(fixed_date())
            .now(fixed_now())
            .build()
            .expect("minimal meal must be valid");

        assert_eq!(meal.name, "Spaghetti");
        assert_eq!(meal.date, fixed_date());
        // Dinner is the default slot.
        assert_eq!(meal.slot, MealSlot::Dinner);
        assert!(meal.cook.is_none());
        assert!(meal.ingredient_lines.is_empty());
        assert!(meal.notes.is_none());
        assert_eq!(meal.created_at, fixed_now());
    }

    #[test]
    fn empty_name_is_rejected() {
        // A blank name is not a valid meal.
        let result = MealBuilder::new("   ")
            .date(fixed_date())
            .now(fixed_now())
            .build();
        assert!(matches!(result, Err(MealError::EmptyName)));
    }

    #[test]
    fn missing_date_is_rejected() {
        // date is required; without it build() must fail.
        let result = MealBuilder::new("Spaghetti").now(fixed_now()).build();
        assert!(matches!(result, Err(MealError::MissingDate)));
    }

    #[test]
    fn missing_now_is_rejected() {
        // now is required so tests stay deterministic and the clock is
        // always injected by the caller.
        let result = MealBuilder::new("Spaghetti").date(fixed_date()).build();
        assert!(matches!(result, Err(MealError::MissingNow)));
    }

    #[test]
    fn ingredients_and_cook_are_recorded() {
        // Builder accumulates ingredient lines and records the cook reference.
        let meal = MealBuilder::new("Tacos")
            .date(fixed_date())
            .slot(MealSlot::Lunch)
            .cook(placeholder_member())
            .ingredient("beef", Some("1 lb".to_owned()))
            .ingredient("tortillas", None)
            .notes("kids' favourite")
            .now(fixed_now())
            .build()
            .unwrap();

        assert_eq!(meal.slot, MealSlot::Lunch);
        assert_eq!(meal.cook, Some(placeholder_member()));
        assert_eq!(meal.ingredient_lines.len(), 2);
        assert_eq!(meal.ingredient_lines[0].name, "beef");
        assert_eq!(meal.ingredient_lines[0].qty.as_deref(), Some("1 lb"));
        assert_eq!(meal.ingredient_lines[1].name, "tortillas");
        assert!(meal.ingredient_lines[1].qty.is_none());
        assert_eq!(meal.notes.as_deref(), Some("kids' favourite"));
    }

    #[test]
    fn meal_slot_round_trips_through_string() {
        // All slot variants must survive Display -> FromStr without loss.
        for slot in [
            MealSlot::Dinner,
            MealSlot::Breakfast,
            MealSlot::Lunch,
            MealSlot::Other,
        ] {
            let s = slot.to_string();
            let parsed: MealSlot = s.parse().expect("known slot string");
            assert_eq!(slot, parsed, "round-trip failed for {slot}");
        }
    }

    #[test]
    fn unknown_slot_returns_error() {
        // Simulates a newer binary writing a slot value an older binary
        // cannot read.
        let result: Result<MealSlot, _> = "brunch".parse();
        assert!(matches!(result, Err(MealError::UnknownSlot(_))));
    }

    #[test]
    fn slot_defaults_to_dinner() {
        // The #[default] attribute on MealSlot must select Dinner.
        assert_eq!(MealSlot::default(), MealSlot::Dinner);
    }
}
