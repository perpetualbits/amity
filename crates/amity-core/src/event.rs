// event.rs — the Event domain type.
//
// Amity is a calendar *aggregator*, not a source of truth (brief §7): external
// calendars own their events; the hub displays them and keeps a small native
// calendar for family-coordination events that belong nowhere else. This module
// models both — the `source` field decides which, and therefore what edit
// affordances the UI offers: full control for native events, override-only for
// read-only external ones (see `event_override.rs`).
//
// This module defines:
//   • `EventSourceKind` — Native | Ics.
//   • `EventSource`     — where the event came from, and whether it is editable.
//   • `Event`           — the event struct (native fields live; some deferred).
//   • `EventBuilder`    — validates invariants before construction.
//   • `EventError`      — builder / parse error variants.
//
// Native recurring events reuse the Task recurrence engine (RRULE subset,
// DST-anchored materialisation) via the `recurrence` field — one engine, already
// tested (ADR-0002). External recurrence is trusted to the source (brief §6.6).
//
// Time semantics (brief §7):
//   • Every event stores its IANA `timezone`; the hub always displays home time
//     (Europe/Amsterdam by default), and storing the zone keeps DST correct.
//   • A timed event has a `start_at` and an optional `end_at` (which must not
//     precede the start). It surfaces on its start date at its clock time.
//   • An all-day event sets `all_day = true`; `start_at` is local midnight and
//     it surfaces on its date without a clock time, leading the day (brief §11).
//   • Values are stored offset-explicit (RFC 3339) and rendered at display time.
//
// Fields DEFERRED (they interact with subsystems that do not exist yet):
//   attendees — needs member management.
//   reminders — needs the notification system.
// Both land in a later task; the columns are added when they do.

// Serde derives for the JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// OffsetDateTime carries the UTC offset explicitly — no naive datetimes.
use time::OffsetDateTime;

// Typed id for this entity.
use crate::ids::EventId;
// RecurrenceRule is optional on Event; the materialiser turns it into instances.
use crate::recurrence::RecurrenceRule;

// ─── EventSourceKind ────────────────────────────────────────────────────────

/// Where an event originates.
///
/// `Native` events are created and owned by the hub — fully editable. `Ics`
/// events come from a read-only external calendar feed (school, sports club,
/// municipal afvalkalender, NL holidays, a personal Google/Apple calendar); the
/// household can overlay changes via `EventOverride` but never writes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSourceKind {
    /// Created and owned by the hub; fully editable.
    Native,
    /// Ingested from a read-only external ICS feed.
    Ics,
}

impl std::fmt::Display for EventSourceKind {
    /// Produce the `snake_case` storage string.
    ///
    /// These strings are the database storage contract; changing them requires a
    /// migration to backfill existing rows. Do not change without an ADR.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match keeps this independent of serde for SQL bind parameters.
        let s = match self {
            Self::Native => "native",
            Self::Ics => "ics",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for EventSourceKind {
    type Err = EventError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::UnknownSourceKind`] for an unrecognised value.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm mirrors the Display impl above for round-trip safety.
        match s {
            "native" => Ok(Self::Native),
            "ics" => Ok(Self::Ics),
            other => Err(EventError::UnknownSourceKind(other.to_owned())),
        }
    }
}

// ─── EventSource ────────────────────────────────────────────────────────────

/// Where an event came from, and whether it can be edited.
///
/// The UI reads `read_only` to decide edit affordances: native events get full
/// controls; read-only external events get create-override-only. The external
/// bookkeeping fields (`external_id`, `calendar_id`, `last_synced_at`) are set
/// by the ICS ingestion path (Task 5) and are `None` for native events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSource {
    /// Native or external (ICS).
    pub kind: EventSourceKind,

    /// The event's id within its external source; `None` for native events.
    pub external_id: Option<String>,

    /// Which external calendar it belongs to; `None` for native events.
    pub calendar_id: Option<String>,

    /// Whether the household may edit the event directly.
    ///
    /// `false` for native events (full control); `true` for external ones,
    /// which are changed only via `EventOverride`.
    pub read_only: bool,

    /// When this event was last refreshed from its external source; `None` for
    /// native events (and until the ICS sync job runs).
    pub last_synced_at: Option<OffsetDateTime>,
}

impl EventSource {
    /// A native, fully-editable source (the default for hub-created events).
    #[must_use]
    pub fn native() -> Self {
        // Native events carry no external bookkeeping and are never read-only.
        Self {
            kind: EventSourceKind::Native,
            external_id: None,
            calendar_id: None,
            read_only: false,
            last_synced_at: None,
        }
    }

    /// A read-only external (ICS) source with its bookkeeping fields.
    ///
    /// Used by the ICS ingestion path (Task 5). External events are always
    /// read-only; edits are recorded as `EventOverride`s, never written back.
    #[must_use]
    pub fn ics(
        external_id: impl Into<String>,
        calendar_id: impl Into<String>,
        last_synced_at: OffsetDateTime,
    ) -> Self {
        // Read-only is intrinsic to an external source — enforced here, not by callers.
        Self {
            kind: EventSourceKind::Ics,
            external_id: Some(external_id.into()),
            calendar_id: Some(calendar_id.into()),
            read_only: true,
            last_synced_at: Some(last_synced_at),
        }
    }
}

// ─── Event ──────────────────────────────────────────────────────────────────

/// A calendar event — native or read-only external.
///
/// An event may be one-shot (no recurrence) or recurring (a `RecurrenceRule`
/// the materialiser turns into instances, like a recurring Task). All-day events
/// carry `all_day = true` and surface on their date without a clock time.
///
/// Construction goes through [`EventBuilder`]. Direct field construction is
/// `pub` so the storage layer can reconstruct events from database rows.
///
/// See `docs/amity_brief.md` §6.5 (Event) and §7 (time semantics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Globally unique, time-ordered identifier.
    pub id: EventId,

    /// One-line title. Must not be empty after trimming.
    pub title: String,

    /// When the event starts. For all-day events this is local midnight.
    pub start_at: OffsetDateTime,

    /// When the event ends; `None` for events with no meaningful end.
    ///
    /// When present it must be at or after `start_at` (the builder enforces this).
    pub end_at: Option<OffsetDateTime>,

    /// True for an all-day event; it surfaces on its date without a clock time.
    pub all_day: bool,

    /// IANA timezone the event is anchored to (e.g. "Europe/Amsterdam").
    ///
    /// The hub always displays home time; storing the zone keeps DST correct.
    pub timezone: String,

    /// Optional free-form location.
    pub location: Option<String>,

    /// Recurrence rule, if this is a recurring native event.
    ///
    /// `None` for one-shot events and for external events (whose recurrence is
    /// trusted to the source). The materialiser generates instances from this.
    pub recurrence: Option<RecurrenceRule>,

    /// Where the event came from and whether it is editable.
    pub source: EventSource,

    /// When this event was first created.
    pub created_at: OffsetDateTime,

    /// When this event was last modified.
    pub updated_at: OffsetDateTime,
}

// ─── EventBuilder ───────────────────────────────────────────────────────────

/// Builder for [`Event`].
///
/// Enforces the invariants the struct requires:
/// - `title` must not be empty after trimming.
/// - `start_at` and `now` must be set.
/// - `end_at`, when set, must be at or after `start_at`.
///
/// Optional fields default sensibly: `timezone` to `"Europe/Amsterdam"` (brief
/// §7), `all_day` to `false`, `source` to a native source, and every `Option`
/// field to `None`.
#[derive(Debug)]
pub struct EventBuilder {
    /// The event title. Required; validated on `build()`.
    title: Option<String>,
    /// Start instant. Required.
    start_at: Option<OffsetDateTime>,
    /// Optional end instant; validated against `start_at` on `build()`.
    end_at: Option<OffsetDateTime>,
    /// All-day flag; defaults to false.
    all_day: bool,
    /// IANA timezone; defaults to Europe/Amsterdam.
    timezone: String,
    /// Optional location.
    location: Option<String>,
    /// Recurrence rule for recurring native events.
    recurrence: Option<RecurrenceRule>,
    /// Event source; defaults to native.
    source: EventSource,
    /// Current wall-clock time for `created_at`/`updated_at`. Required.
    now: Option<OffsetDateTime>,
}

impl EventBuilder {
    /// Start building a new event.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Required fields start absent; build() checks for them.
            title: None,
            start_at: None,
            // Optional end; None means "no meaningful end".
            end_at: None,
            // Timed by default.
            all_day: false,
            // Home time by default (brief §7).
            timezone: "Europe/Amsterdam".to_owned(),
            // No location unless set.
            location: None,
            // One-shot by default.
            recurrence: None,
            // Native, fully-editable by default.
            source: EventSource::native(),
            // now must be supplied for deterministic timestamps.
            now: None,
        }
    }

    /// Set the event title (stored verbatim; validated non-empty on build).
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        // Store as-is; the non-empty check happens in build().
        self.title = Some(title.into());
        self
    }

    /// Set the start instant (required).
    #[must_use]
    pub fn start_at(mut self, start_at: OffsetDateTime) -> Self {
        // Wrap in Some; absence triggers MissingField in build().
        self.start_at = Some(start_at);
        self
    }

    /// Set the end instant (must be at or after `start_at`).
    #[must_use]
    pub fn end_at(mut self, end_at: OffsetDateTime) -> Self {
        // Wrap in Some; build() checks end >= start.
        self.end_at = Some(end_at);
        self
    }

    /// Mark this as an all-day event.
    #[must_use]
    pub fn all_day(mut self, all_day: bool) -> Self {
        // Overrides the timed default; all-day events show no clock time.
        self.all_day = all_day;
        self
    }

    /// Override the timezone (defaults to Europe/Amsterdam).
    #[must_use]
    pub fn timezone(mut self, timezone: impl Into<String>) -> Self {
        // Replaces the Amsterdam default; build() rejects a blank value.
        self.timezone = timezone.into();
        self
    }

    /// Set an optional location.
    #[must_use]
    pub fn location(mut self, location: impl Into<String>) -> Self {
        // Wrap in Some; None means "no location recorded".
        self.location = Some(location.into());
        self
    }

    /// Attach a recurrence rule (makes this a recurring event).
    #[must_use]
    pub fn recurrence(mut self, rule: RecurrenceRule) -> Self {
        // Wrap in Some; None means a one-shot event.
        self.recurrence = Some(rule);
        self
    }

    /// Set the event source (defaults to native).
    #[must_use]
    pub fn source(mut self, source: EventSource) -> Self {
        // Overrides the native default (e.g. for an ingested ICS event).
        self.source = source;
        self
    }

    /// Supply the current wall-clock time for the audit timestamps (required).
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        // Production callers pass OffsetDateTime::now_utc(); tests pass a fixed value.
        self.now = Some(now);
        self
    }

    /// Validate all invariants and construct the `Event`.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::EmptyTitle`] if the title is blank; [`EventError::
    /// MissingField`] if `start_at` or `now` is unset; [`EventError::EndBeforeStart`]
    /// if `end_at` precedes `start_at`; [`EventError::EmptyTimezone`] if blank.
    pub fn build(self) -> Result<Event, EventError> {
        // Title first — the most visible required field.
        let title = self
            .title
            .ok_or_else(|| EventError::MissingField("title".to_owned()))?;
        // Reject a blank title; whitespace is not a title.
        if title.trim().is_empty() {
            return Err(EventError::EmptyTitle);
        }

        // start_at and now are both required for a complete event.
        let start_at = self
            .start_at
            .ok_or_else(|| EventError::MissingField("start_at".to_owned()))?;
        let now = self
            .now
            .ok_or_else(|| EventError::MissingField("now".to_owned()))?;

        // The timezone must name a real zone; an empty string is a bug.
        if self.timezone.trim().is_empty() {
            return Err(EventError::EmptyTimezone);
        }

        // An end, when present, cannot precede the start.
        if let Some(end) = self.end_at
            && end < start_at
        {
            return Err(EventError::EndBeforeStart);
        }

        // Generate a fresh time-ordered id at construction time.
        let id = EventId::new();

        // Both timestamps start equal to `now`; updates bump only updated_at.
        Ok(Event {
            // Freshly generated above.
            id,
            // Validated non-empty; stored verbatim.
            title,
            // Required start instant.
            start_at,
            // Optional end (already checked >= start).
            end_at: self.end_at,
            // Timed vs all-day.
            all_day: self.all_day,
            // Validated non-empty IANA zone.
            timezone: self.timezone,
            // Optional free-form location.
            location: self.location,
            // Some for recurring native events.
            recurrence: self.recurrence,
            // Native by default, or an ingested external source.
            source: self.source,
            // created_at is set once and never changes.
            created_at: now,
            // updated_at starts at now; storage bumps it on update.
            updated_at: now,
        })
    }
}

impl Default for EventBuilder {
    /// Returns a builder with default optional values and no required fields set.
    fn default() -> Self {
        // Delegate to new() — single source of truth for the initial state.
        Self::new()
    }
}

// ─── EventError ─────────────────────────────────────────────────────────────

/// Errors produced when building or parsing `Event`-related types.
///
/// The builder-validation variants (`EmptyTitle`, `MissingField`,
/// `EndBeforeStart`, `EmptyTimezone`) are client errors the service maps to HTTP
/// 422. The `Unknown*` variants indicate a stored string the current binary does
/// not recognise — a data-migration gap or a storage bug, not user input.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// `title` was present but contained only whitespace.
    ///
    /// The service layer maps this to HTTP 422.
    #[error("event title must not be empty")]
    EmptyTitle,

    /// A required builder field (`start_at` or `now`) was not set.
    ///
    /// The enclosed string names the field so callers can report which.
    #[error("required field not set: {0}")]
    MissingField(String),

    /// `end_at` was earlier than `start_at`.
    ///
    /// An event cannot end before it begins; the builder rejects this.
    #[error("event end must be at or after its start")]
    EndBeforeStart,

    /// The timezone string was empty.
    ///
    /// Every event must name a real IANA zone so the hub can render home time.
    #[error("event timezone must not be empty")]
    EmptyTimezone,

    /// The database contained an unrecognised `source_kind` string.
    ///
    /// Only `native` and `ics` are known; anything else is a storage-level bug.
    #[error("unknown event source kind: {0}")]
    UnknownSourceKind(String),

    /// The database contained an unrecognised `EventOverride` action string.
    ///
    /// Only `cancel`, `reschedule`, and `annotate` are known.
    #[error("unknown event override action: {0}")]
    UnknownOverrideAction(String),
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn now() -> OffsetDateTime {
        datetime!(2026-07-25 10:00:00 UTC)
    }

    fn start() -> OffsetDateTime {
        datetime!(2026-07-26 09:00:00 UTC)
    }

    fn minimal() -> Event {
        EventBuilder::new()
            .title("school play")
            .start_at(start())
            .now(now())
            .build()
            .expect("minimal event must be valid")
    }

    #[test]
    fn build_minimal_event() {
        let event = minimal();
        assert_eq!(event.title, "school play");
        assert_eq!(event.start_at, start());
        // Defaults: no end, timed, native, Amsterdam, no recurrence.
        assert!(event.end_at.is_none());
        assert!(!event.all_day);
        assert_eq!(event.timezone, "Europe/Amsterdam");
        assert_eq!(event.source.kind, EventSourceKind::Native);
        assert!(!event.source.read_only);
        assert!(event.recurrence.is_none());
        // Timestamps both equal `now`.
        assert_eq!(event.created_at, now());
        assert_eq!(event.updated_at, now());
    }

    #[test]
    fn empty_title_is_rejected() {
        let result = EventBuilder::new()
            .title("   ")
            .start_at(start())
            .now(now())
            .build();
        assert!(matches!(result, Err(EventError::EmptyTitle)));
    }

    #[test]
    fn missing_start_is_rejected() {
        let result = EventBuilder::new().title("x").now(now()).build();
        assert!(matches!(result, Err(EventError::MissingField(_))));
    }

    #[test]
    fn missing_now_is_rejected() {
        let result = EventBuilder::new().title("x").start_at(start()).build();
        assert!(matches!(result, Err(EventError::MissingField(_))));
    }

    #[test]
    fn end_before_start_is_rejected() {
        let result = EventBuilder::new()
            .title("x")
            .start_at(start())
            // One hour before the start — invalid.
            .end_at(start() - time::Duration::hours(1))
            .now(now())
            .build();
        assert!(matches!(result, Err(EventError::EndBeforeStart)));
    }

    #[test]
    fn empty_timezone_is_rejected() {
        let result = EventBuilder::new()
            .title("x")
            .start_at(start())
            .timezone("  ")
            .now(now())
            .build();
        assert!(matches!(result, Err(EventError::EmptyTimezone)));
    }

    #[test]
    fn all_day_event_carries_the_flag() {
        let event = EventBuilder::new()
            .title("king's day")
            .start_at(datetime!(2026-04-27 00:00:00 UTC))
            .all_day(true)
            .now(now())
            .build()
            .unwrap();
        assert!(event.all_day);
    }

    #[test]
    fn recurring_event_carries_rule() {
        let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=MO", "Europe/Amsterdam");
        let event = EventBuilder::new()
            .title("bin day")
            .start_at(start())
            .recurrence(rule.clone())
            .now(now())
            .build()
            .unwrap();
        assert_eq!(event.recurrence, Some(rule));
    }

    #[test]
    fn external_source_is_read_only() {
        let source = EventSource::ics("ext-123", "school-cal", now());
        let event = EventBuilder::new()
            .title("term starts")
            .start_at(start())
            .source(source)
            .now(now())
            .build()
            .unwrap();
        assert_eq!(event.source.kind, EventSourceKind::Ics);
        assert!(event.source.read_only);
        assert_eq!(event.source.external_id.as_deref(), Some("ext-123"));
    }

    #[test]
    fn source_kind_round_trips_through_string() {
        for kind in [EventSourceKind::Native, EventSourceKind::Ics] {
            let s = kind.to_string();
            let parsed: EventSourceKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
        assert!("bogus".parse::<EventSourceKind>().is_err());
    }
}
