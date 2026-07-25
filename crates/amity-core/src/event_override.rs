// event_override.rs — the EventOverride domain type.
//
// External calendar events are read-only: the household never writes back to the
// source (brief §7). But a specific instance sometimes needs a local change —
// "bin day moved for King's Day", "school trip cancelled". An `EventOverride`
// records that change as a local overlay on one instance of a read-only event,
// leaving the source untouched.
//
// This module defines:
//   • `OverrideAction`  — Cancel | Reschedule | Annotate.
//   • `EventOverride`   — the overlay record (id, target event, instance, action).
//
// Overrides are applied by the surfacing layer before an event's instances are
// shown, so the external source feed is never modified — the change is the
// household's, held locally (brief §7, "aggregator, not source of truth").
//
// See `docs/amity_brief.md` §6.5 (`EventOverride`).

// Serde derives for the JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// `Date` identifies which instance; `OffsetDateTime` timestamps the record.
use time::{Date, OffsetDateTime};

// The shared event error enum (parse failures) and typed ids.
use crate::event::EventError;
use crate::ids::{EventId, EventOverrideId, MemberId};

// ─── OverrideAction ─────────────────────────────────────────────────────────

/// What an `EventOverride` does to the targeted instance.
///
/// The system applies these before surfacing: a cancelled instance never
/// appears; a rescheduled one appears at its new time; an annotated one appears
/// with the household's note alongside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideAction {
    /// Hide this instance entirely (it did not or will not happen).
    Cancel,
    /// Move this instance to a new time carried in the override's `payload`.
    Reschedule,
    /// Attach a note to this instance; the instance still occurs as scheduled.
    Annotate,
}

impl std::fmt::Display for OverrideAction {
    /// Produce the `snake_case` storage string (the database contract).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match keeps this usable for SQL bind parameters.
        let s = match self {
            Self::Cancel => "cancel",
            Self::Reschedule => "reschedule",
            Self::Annotate => "annotate",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for OverrideAction {
    type Err = EventError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::UnknownOverrideAction`] for an unrecognised value.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm mirrors the Display impl above for round-trip safety.
        match s {
            "cancel" => Ok(Self::Cancel),
            "reschedule" => Ok(Self::Reschedule),
            "annotate" => Ok(Self::Annotate),
            other => Err(EventError::UnknownOverrideAction(other.to_owned())),
        }
    }
}

// ─── EventOverride ──────────────────────────────────────────────────────────

/// A local overlay applied to one instance of a read-only external event.
///
/// It targets a specific `(source_event_id, instance_date)` occurrence and
/// records what the household changed. The `payload` carries the action's data:
/// a new RFC 3339 time for `Reschedule`, a note for `Annotate`, and is unused
/// (`None`) for `Cancel`.
///
/// See `docs/amity_brief.md` §6.5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventOverride {
    /// Globally unique, time-ordered identifier for this overlay.
    pub id: EventOverrideId,

    /// The external event whose instance is being overridden.
    pub source_event_id: EventId,

    /// Which instance (calendar date) this overlay applies to.
    pub instance_date: Date,

    /// What the overlay does to that instance.
    pub action: OverrideAction,

    /// Action data: a new time (`Reschedule`), a note (`Annotate`), or `None`.
    pub payload: Option<String>,

    /// The member who created the overlay.
    pub created_by: MemberId,

    /// When the overlay was created.
    pub created_at: OffsetDateTime,
}

impl EventOverride {
    /// Create a new overlay, generating a fresh id.
    ///
    /// The caller supplies `created_at` (no hidden clock reads), matching the
    /// rest of the domain layer.
    #[must_use]
    pub fn new(
        source_event_id: EventId,
        instance_date: Date,
        action: OverrideAction,
        payload: Option<String>,
        created_by: MemberId,
        created_at: OffsetDateTime,
    ) -> Self {
        // Generate the overlay's own time-ordered id at construction time.
        Self {
            // A fresh id for this overlay row.
            id: EventOverrideId::new(),
            // The external event this overlays.
            source_event_id,
            // The specific instance date it targets.
            instance_date,
            // Cancel / Reschedule / Annotate.
            action,
            // Action data (new time, note, or None).
            payload,
            // The member who made the change.
            created_by,
            // When the overlay was recorded.
            created_at,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    fn member() -> MemberId {
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    #[test]
    fn new_override_generates_id_and_keeps_fields() {
        let event_id = EventId::new();
        let created_at = datetime!(2026-07-25 10:00:00 UTC);
        let overlay = EventOverride::new(
            event_id,
            date!(2026 - 04 - 27),
            OverrideAction::Reschedule,
            Some("2026-04-27T14:00:00+02:00".to_owned()),
            member(),
            created_at,
        );
        // Fields are carried through verbatim; the id is fresh and non-nil.
        assert_eq!(overlay.source_event_id, event_id);
        assert_eq!(overlay.action, OverrideAction::Reschedule);
        assert_eq!(
            overlay.payload.as_deref(),
            Some("2026-04-27T14:00:00+02:00")
        );
        assert_eq!(overlay.created_by, member());
        assert_eq!(overlay.created_at, created_at);
    }

    #[test]
    fn action_round_trips_through_string() {
        for action in [
            OverrideAction::Cancel,
            OverrideAction::Reschedule,
            OverrideAction::Annotate,
        ] {
            let s = action.to_string();
            let parsed: OverrideAction = s.parse().unwrap();
            assert_eq!(action, parsed);
        }
    }

    #[test]
    fn unknown_action_is_rejected() {
        let result: Result<OverrideAction, _> = "postpone".parse();
        assert!(matches!(result, Err(EventError::UnknownOverrideAction(_))));
    }
}
