// member.rs — the Member domain type.
//
// A Member is a DISPLAY REGISTRY ENTRY ONLY: a name (and optionally an
// initial and a colour) so the hub can show "who" next to a task, meal, or
// completion log entry. That is the entire entity — nothing else lives here.
//
// Explicitly REFUSED, by design (mirrors the philosophy's anti-surveillance
// stance — this entity is kept too small to host tracking):
//   • accounts / authentication / passwords — Amity has no login concept.
//   • per-member devices or sessions — there is no "signed in as".
//   • roles or permissions — every household member can do everything.
//   • activity, statistics, or presence — no "last seen", no streaks.
//   • any behavioural data of any kind.
//   • any age or child-specific structure — a child is a Member exactly like
//     an adult; there is no `is_child` flag or separate child type.
// Do NOT add any field beyond {id, display_name, initial, color, created_at}
// without a new ADR revisiting this boundary.
//
// This module defines:
//   • `MemberColor`  — enum: Sage, Clay, Ochre, Slate, Plum, Teal. Optional —
//                       no default, since a member without a chosen colour is
//                       simply unhighlighted, not an error.
//   • `Member`        — the domain struct (fields pub for storage
//                        reconstruction, mirroring `PantryItem`/`Task`).
//   • `MemberBuilder`  — builder that validates invariants before construction.
//   • `MemberError`    — error variants for builder and parse failures.

// Serde derives for JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// OffsetDateTime for `created_at`.
use time::OffsetDateTime;

// Typed id for this entity.
use crate::ids::MemberId;

// ─── MemberColor ─────────────────────────────────────────────────────────────

/// An optional accent colour used to distinguish members at a glance in the
/// hub UI (avatars, badges). Purely cosmetic — the system attaches no meaning
/// to a member's colour beyond "how they are drawn".
// Copy: colours are small enough to pass by value everywhere they are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberColor {
    /// Muted green.
    Sage,
    /// Warm terracotta.
    Clay,
    /// Earthy yellow-orange.
    Ochre,
    /// Cool blue-grey.
    Slate,
    /// Muted purple.
    Plum,
    /// Blue-green.
    Teal,
}

impl std::fmt::Display for MemberColor {
    /// Produce the `snake_case` storage string.
    ///
    /// These strings are the database storage contract. Changing them requires
    /// a migration to backfill existing rows. Do not change without an ADR.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match keeps this independent of serde so it can be used in
        // non-serde contexts (e.g. SQL bind parameters).
        let s = match self {
            Self::Sage => "sage",
            Self::Clay => "clay",
            Self::Ochre => "ochre",
            Self::Slate => "slate",
            Self::Plum => "plum",
            Self::Teal => "teal",
        };
        // Write the selected storage string.
        write!(f, "{s}")
    }
}

impl std::str::FromStr for MemberColor {
    type Err = MemberError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns `MemberError::UnknownColor` if the string is not a known variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm must mirror the Display impl above to ensure round-trip safety.
        match s {
            "sage" => Ok(Self::Sage),
            "clay" => Ok(Self::Clay),
            "ochre" => Ok(Self::Ochre),
            "slate" => Ok(Self::Slate),
            "plum" => Ok(Self::Plum),
            "teal" => Ok(Self::Teal),
            other => Err(MemberError::UnknownColor(other.to_owned())),
        }
    }
}

// ─── Member ───────────────────────────────────────────────────────────────────

/// A household member — a display registry entry, nothing more.
///
/// See the module doc above for the hard boundary on what this entity is
/// allowed to hold. Construction goes through [`MemberBuilder`]. Direct field
/// construction is `pub` so the storage layer can reconstruct members from
/// database rows — callers outside the storage layer should use the builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    /// Globally unique, time-ordered identifier.
    pub id: MemberId,
    /// The name shown throughout the hub. Must not be empty after trimming.
    pub display_name: String,
    /// Optional short label (e.g. a single letter) for compact avatar display.
    pub initial: Option<String>,
    /// Optional accent colour for the member's avatar/badge. Purely cosmetic.
    pub color: Option<MemberColor>,
    /// When this member was first registered.
    pub created_at: OffsetDateTime,
}

// ─── MemberBuilder ───────────────────────────────────────────────────────────

/// Builder for [`Member`].
///
/// Enforces the invariants the struct requires:
/// - `display_name` must not be empty after trimming.
/// - `now` must be set (used for `created_at`).
#[derive(Debug)]
pub struct MemberBuilder {
    /// The member's display name. Required; validated on `build()`.
    display_name: Option<String>,
    /// Optional short label for compact display.
    initial: Option<String>,
    /// Optional accent colour.
    color: Option<MemberColor>,
    /// Caller-supplied id, used by the storage layer to reconstruct a row
    /// with its existing id rather than minting a fresh one.
    id: Option<MemberId>,
    /// Current wall-clock time for `created_at`. Required.
    now: Option<OffsetDateTime>,
}

impl MemberBuilder {
    /// Start building a new member with the given display name.
    #[must_use]
    pub fn new(display_name: impl Into<String>) -> Self {
        Self {
            // Store as-is; the non-empty check happens in build().
            display_name: Some(display_name.into()),
            // No initial unless set.
            initial: None,
            // No colour unless set — a member need not have one.
            color: None,
            // No caller-supplied id unless the storage layer sets one.
            id: None,
            // now must be supplied for deterministic timestamps.
            now: None,
        }
    }

    /// Set an optional short label (e.g. a single letter) for compact display.
    #[must_use]
    pub fn initial(mut self, initial: impl Into<String>) -> Self {
        // Wrap in Some to signal that an initial was provided.
        self.initial = Some(initial.into());
        self
    }

    /// Set an optional accent colour.
    #[must_use]
    pub fn color(mut self, color: MemberColor) -> Self {
        // Wrap in Some to signal that a colour was chosen.
        self.color = Some(color);
        self
    }

    /// Supply a specific id (used by the storage layer to reconstruct a row
    /// with its existing id). Production callers creating a brand-new member
    /// should not call this — `build()` mints a fresh id when absent.
    #[must_use]
    pub fn id(mut self, id: MemberId) -> Self {
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

    /// Validate all invariants and construct the `Member`.
    ///
    /// # Errors
    ///
    /// Returns [`MemberError::EmptyName`] if the display name is blank.
    /// Returns [`MemberError::MissingNow`] if `now` was not set.
    pub fn build(self) -> Result<Member, MemberError> {
        // display_name is set unconditionally in new(); validate its trimmed form.
        let display_name = self.display_name.unwrap_or_default();
        if display_name.trim().is_empty() {
            return Err(MemberError::EmptyName);
        }

        // now is required for a complete Member.
        let now = self.now.ok_or(MemberError::MissingNow)?;

        // Use the caller-supplied id (storage reconstruction) or mint a
        // fresh time-ordered one (new-member construction).
        let id = self.id.unwrap_or_default();

        Ok(Member {
            // Caller-supplied or freshly minted above.
            id,
            // Already validated non-empty above.
            display_name,
            // Both optional fields pass through unchanged.
            initial: self.initial,
            color: self.color,
            // Only field derived from `now` — Member has no `updated_at`.
            created_at: now,
        })
    }
}

// ─── MemberError ────────────────────────────────────────────────────────────

/// Errors produced when building a `Member` or parsing `MemberColor`.
#[derive(Debug, thiserror::Error)]
pub enum MemberError {
    /// `display_name` was absent or contained only whitespace.
    #[error("member display name must not be empty")]
    EmptyName,

    /// `now` was not set on the builder.
    #[error("required field not set: now")]
    MissingNow,

    /// The database (or a client request) contained an unrecognised `color`
    /// string.
    #[error("unknown member color: {0}")]
    UnknownColor(String),
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Fixed "now" for deterministic tests.
    fn fixed_now() -> OffsetDateTime {
        datetime!(2026-08-24 10:00:00 UTC)
    }

    #[test]
    fn build_minimal_member() {
        // Happy path: only the required field (display_name) plus now.
        let member = MemberBuilder::new("Alice")
            .now(fixed_now())
            .build()
            .expect("minimal member must be valid");

        assert_eq!(member.display_name, "Alice");
        assert!(member.initial.is_none());
        assert!(member.color.is_none());
        assert_eq!(member.created_at, fixed_now());
    }

    #[test]
    fn empty_display_name_is_rejected() {
        // A blank name is not a valid member — an accidental tap should not
        // create an invisible registry entry.
        let result = MemberBuilder::new("   ").now(fixed_now()).build();
        assert!(matches!(result, Err(MemberError::EmptyName)));
    }

    #[test]
    fn missing_now_is_rejected() {
        // now is required so tests stay deterministic and the clock is
        // always injected by the caller (never read inside amity-core).
        let result = MemberBuilder::new("Alice").build();
        assert!(matches!(result, Err(MemberError::MissingNow)));
    }

    #[test]
    fn initial_and_color_are_recorded_when_provided() {
        // Both optional fields must round-trip through the builder unchanged.
        let member = MemberBuilder::new("Ben")
            .initial("B")
            .color(MemberColor::Clay)
            .now(fixed_now())
            .build()
            .unwrap();
        assert_eq!(member.initial.as_deref(), Some("B"));
        assert_eq!(member.color, Some(MemberColor::Clay));
    }

    #[test]
    fn member_color_round_trips_through_string() {
        // All colour variants must survive Display→FromStr without loss —
        // this guards the storage contract the same way TaskStatus's test does.
        for color in [
            MemberColor::Sage,
            MemberColor::Clay,
            MemberColor::Ochre,
            MemberColor::Slate,
            MemberColor::Plum,
            MemberColor::Teal,
        ] {
            let s = color.to_string();
            let parsed: MemberColor = s.parse().expect("known color string");
            assert_eq!(color, parsed, "round-trip failed for {color}");
        }
    }

    #[test]
    fn unknown_color_returns_error() {
        // Simulates a newer binary writing a colour value an older binary
        // cannot read, or a malformed client request.
        let result: Result<MemberColor, _> = "burgundy".parse();
        assert!(matches!(result, Err(MemberError::UnknownColor(_))));
    }
}
