// pantry.rs — the PantryItem domain type.
//
// PantryItem is a deliberately lightweight "staples memory": a name the
// household has told Amity it usually keeps on hand, plus an optional note.
// It exists solely so `grocery::plan_grocery_additions` can skip suggesting
// ingredients the household already stocks (brief: P2 Slice 1 scope).
//
// NOT modelled in this phase (by design — see the P2 Slice 1 brief):
//   • stock levels / quantities — there is no "how much is left".
//   • low-stock thresholds — there is no depletion tracking or restock alert.
//   • purchase history — this is a static list of names, not a ledger.
// Those would require actual inventory tracking, which is explicitly deferred.
//
// This module defines:
//   • `PantryItem`        — the domain struct (fields pub for storage
//                            reconstruction, mirroring `Task`/`Event`/`Meal`).
//   • `PantryItemBuilder` — builder that validates invariants before construction.
//   • `PantryError`       — error variants for builder failures.

// Serde derives for JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// OffsetDateTime for `created_at`.
use time::OffsetDateTime;

// Typed id for this entity.
use crate::ids::PantryItemId;

// ─── PantryItem ─────────────────────────────────────────────────────────────

/// A staple the household usually keeps on hand.
///
/// Deliberately lightweight: just a name and an optional note. No quantity,
/// no threshold, no depletion tracking — see the module doc for why.
///
/// Construction goes through [`PantryItemBuilder`]. Direct field construction
/// is `pub` so the storage layer can reconstruct items from database rows —
/// callers outside the storage layer should use the builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PantryItem {
    /// Globally unique, time-ordered identifier.
    pub id: PantryItemId,
    /// The staple's name (e.g. "flour"). Must not be empty after trimming.
    ///
    /// Matched case-insensitively against ingredient names by
    /// `grocery::plan_grocery_additions` — stored verbatim here, normalised
    /// only at comparison time.
    pub name: String,
    /// Optional free-form note (e.g. "keep two bags, we bake a lot").
    pub note: Option<String>,
    /// When this pantry item was first recorded.
    pub created_at: OffsetDateTime,
}

// ─── PantryItemBuilder ──────────────────────────────────────────────────────

/// Builder for [`PantryItem`].
///
/// Enforces the invariants the struct requires:
/// - `name` must not be empty after trimming.
/// - `now` must be set (used for `created_at`).
#[derive(Debug)]
pub struct PantryItemBuilder {
    /// The item's name. Required; validated on `build()`.
    name: Option<String>,
    /// Optional free-form note.
    note: Option<String>,
    /// Caller-supplied id, used by the storage layer to reconstruct a row
    /// with its existing id rather than minting a fresh one.
    id: Option<PantryItemId>,
    /// Current wall-clock time for `created_at`. Required.
    now: Option<OffsetDateTime>,
}

impl PantryItemBuilder {
    /// Start building a new pantry item with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            // Store as-is; the non-empty check happens in build().
            name: Some(name.into()),
            // No note unless set.
            note: None,
            // No caller-supplied id unless the storage layer sets one.
            id: None,
            // now must be supplied for deterministic timestamps.
            now: None,
        }
    }

    /// Set an optional free-form note.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        // Wrap in Some to signal that a note was provided.
        self.note = Some(note.into());
        self
    }

    /// Supply a specific id (used by the storage layer to reconstruct a row
    /// with its existing id). Production callers creating a brand-new item
    /// should not call this — `build()` mints a fresh id when absent.
    #[must_use]
    pub fn id(mut self, id: PantryItemId) -> Self {
        self.id = Some(id);
        self
    }

    /// Supply the current wall-clock time for `created_at` (required).
    ///
    /// Production callers pass `OffsetDateTime::now_utc()`; tests pass a
    /// fixed value for determinism.
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Validate all invariants and construct the `PantryItem`.
    ///
    /// # Errors
    ///
    /// Returns [`PantryError::EmptyName`] if the name is blank.
    /// Returns [`PantryError::MissingNow`] if `now` was not set.
    pub fn build(self) -> Result<PantryItem, PantryError> {
        // Name is set unconditionally in new(); validate its trimmed form.
        let name = self.name.unwrap_or_default();
        if name.trim().is_empty() {
            return Err(PantryError::EmptyName);
        }

        // now is required for a complete PantryItem.
        let now = self.now.ok_or(PantryError::MissingNow)?;

        // Use the caller-supplied id (storage reconstruction) or mint a
        // fresh time-ordered one (new-item construction).
        let id = self.id.unwrap_or_default();

        Ok(PantryItem {
            id,
            name,
            note: self.note,
            created_at: now,
        })
    }
}

// ─── PantryError ────────────────────────────────────────────────────────────

/// Errors produced when building a `PantryItem`.
#[derive(Debug, thiserror::Error)]
pub enum PantryError {
    /// `name` was absent or contained only whitespace.
    #[error("pantry item name must not be empty")]
    EmptyName,

    /// `now` was not set on the builder.
    #[error("required field not set: now")]
    MissingNow,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Fixed "now" for deterministic tests.
    fn fixed_now() -> OffsetDateTime {
        datetime!(2026-08-22 10:00:00 UTC)
    }

    #[test]
    fn build_minimal_pantry_item() {
        // Happy path: only the required fields (name, now) are set.
        let item = PantryItemBuilder::new("Flour")
            .now(fixed_now())
            .build()
            .expect("minimal pantry item must be valid");

        assert_eq!(item.name, "Flour");
        assert!(item.note.is_none());
        assert_eq!(item.created_at, fixed_now());
    }

    #[test]
    fn empty_name_is_rejected() {
        // A blank name is not a valid pantry item.
        let result = PantryItemBuilder::new("   ").now(fixed_now()).build();
        assert!(matches!(result, Err(PantryError::EmptyName)));
    }

    #[test]
    fn missing_now_is_rejected() {
        // now is required so tests stay deterministic and the clock is
        // always injected by the caller.
        let result = PantryItemBuilder::new("Flour").build();
        assert!(matches!(result, Err(PantryError::MissingNow)));
    }

    #[test]
    fn note_is_recorded_when_provided() {
        // The optional note must round-trip through the builder unchanged.
        let item = PantryItemBuilder::new("Rice")
            .note("always keep 2kg")
            .now(fixed_now())
            .build()
            .unwrap();
        assert_eq!(item.note.as_deref(), Some("always keep 2kg"));
    }
}
