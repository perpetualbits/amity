// calendar.rs — the Calendar domain type: one subscribed external ICS feed.
//
// Amity is a calendar aggregator (brief §7): the household subscribes to
// read-only ICS feeds — a school calendar per child, sports clubs, the
// municipal afvalkalender, NL holidays, personal Google/Apple calendars — and
// the hub displays their events. A `Calendar` is the subscription: a name, a
// feed URL, a category, and whether it is currently synced. The runtime sync
// health (last status, last error, event count) is a separate concern kept in
// `CalendarSyncState`, updated by the sync job, so the entity stays a clean
// description of the subscription rather than its moment-to-moment status.
//
// Construction goes through `CalendarBuilder` with an injected `now`, matching
// every other entity in the codebase. The feed URL is normalised at build time:
// `webcal://` is rewritten to `https://`, and any non-http(s) scheme is rejected
// so a feed can never point at a local file or an unexpected protocol.

// Serde derives for the JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// OffsetDateTime is the canonical timestamp; RFC 3339 is the storage form.
use time::OffsetDateTime;

// The typed id for this entity.
use crate::ids::CalendarId;

// ─── CalendarCategory ─────────────────────────────────────────────────────────

/// The kind of feed, used for grouping and (later) display treatment.
///
/// Categories are advisory — they never change how a feed is fetched or parsed,
/// only how the UI may group it. `Other` is the safe default for a feed that
/// fits none of the named kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CalendarCategory {
    /// A child's school calendar.
    School,
    /// A sports or hobby club.
    Club,
    /// Municipal waste collection (the afvalkalender).
    Waste,
    /// Public holidays.
    Holiday,
    /// An adult member's personal calendar (Google/Apple, via its ICS URL).
    Personal,
    /// Anything else.
    #[default]
    Other,
}

impl std::fmt::Display for CalendarCategory {
    /// Produce the `snake_case` storage string (the database contract).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A manual match keeps the storage strings independent of serde.
        let s = match self {
            Self::School => "school",
            Self::Club => "club",
            Self::Waste => "waste",
            Self::Holiday => "holiday",
            Self::Personal => "personal",
            Self::Other => "other",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for CalendarCategory {
    type Err = CalendarError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`CalendarError::UnknownCategory`] for an unrecognised value.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm mirrors the Display impl above for round-trip safety.
        match s {
            "school" => Ok(Self::School),
            "club" => Ok(Self::Club),
            "waste" => Ok(Self::Waste),
            "holiday" => Ok(Self::Holiday),
            "personal" => Ok(Self::Personal),
            "other" => Ok(Self::Other),
            other => Err(CalendarError::UnknownCategory(other.to_owned())),
        }
    }
}

// ─── SyncStatus ───────────────────────────────────────────────────────────────

/// The outcome of the most recent sync attempt for a feed.
///
/// `Never` means the feed has not been synced yet (freshly added). The others
/// let the API and operator see why a feed is stale without reading logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    /// Not yet synced (a freshly added feed).
    #[default]
    Never,
    /// The last sync fetched and parsed cleanly.
    Ok,
    /// The last fetch failed (network, timeout, oversize, non-2xx).
    Unreachable,
    /// The feed was fetched but is not valid iCalendar.
    ParseError,
}

impl std::fmt::Display for SyncStatus {
    /// Produce the `snake_case` storage string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match, matching the Display/FromStr convention used elsewhere.
        let s = match self {
            Self::Never => "never",
            Self::Ok => "ok",
            Self::Unreachable => "unreachable",
            Self::ParseError => "parse_error",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for SyncStatus {
    type Err = CalendarError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`CalendarError::UnknownSyncStatus`] for an unrecognised value.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Mirror the Display impl for round-trip safety.
        match s {
            "never" => Ok(Self::Never),
            "ok" => Ok(Self::Ok),
            "unreachable" => Ok(Self::Unreachable),
            "parse_error" => Ok(Self::ParseError),
            other => Err(CalendarError::UnknownSyncStatus(other.to_owned())),
        }
    }
}

// ─── Calendar & sync state ────────────────────────────────────────────────────

/// A subscribed external calendar feed.
///
/// The immutable description of a subscription. Sync health lives separately in
/// [`CalendarSyncState`]. Fields are `pub` so the storage layer can reconstruct
/// a `Calendar` from a database row without going through the builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Stable identifier — a time-ordered UUID v7 for sorting and indexing.
    pub id: CalendarId,
    /// Human-facing display name (non-empty, trimmed).
    /// Shown in the UI for selection, filtering, and diagnostics.
    pub name: String,
    /// The feed URL — always `http`/`https` after normalisation.
    /// Never `file://` or other unsafe schemes (validated at build time).
    pub url: String,
    /// Advisory grouping category (School, Club, Waste, etc.).
    /// Used for UI grouping and display treatment; does not affect fetch logic.
    pub category: CalendarCategory,
    /// Whether the sync job fetches this feed. Disabled feeds are skipped.
    /// Allows archiving without deletion; re-enabling resumes normal syncing.
    pub enabled: bool,
    /// When the subscription was created.
    /// Used for audit, sorting, and migration tracking.
    pub created_at: OffsetDateTime,
}

/// The runtime sync health of a feed, updated by the sync job.
///
/// Kept out of [`Calendar`] so the entity is a stable description, not a
/// mutable status record. The repository returns both together in its read
/// model. This struct captures the most recent sync attempt and its outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarSyncState {
    /// When the last successful sync completed; `None` until the first success.
    /// This timestamp tracks the most recent fetch-and-parse that succeeded.
    /// Used to detect stale feeds and calculate sync intervals.
    pub last_synced_at: Option<OffsetDateTime>,
    /// The outcome of the most recent attempt (Never, Ok, Unreachable, `ParseError`).
    /// Lets the API and operators see why a feed is stale without reading logs.
    /// Useful for diagnostics and prioritising which feeds to retry.
    pub last_status: SyncStatus,
    /// A short diagnostic when the last attempt failed; `None` otherwise.
    /// Stores a brief reason (network timeout, non-2xx status, bad iCalendar).
    /// Helps operators debug and prioritise fixing feeds.
    pub last_error: Option<String>,
    /// How many events the last good sync produced.
    /// Lets the UI show a count of imported events and detect if a feed
    /// stopped producing events (a sign of misconfiguration or deletion).
    pub event_count: u32,
}

impl Default for CalendarSyncState {
    /// A never-synced feed: no timestamp, `Never` status, no error, zero events.
    fn default() -> Self {
        Self {
            last_synced_at: None,
            last_status: SyncStatus::Never,
            last_error: None,
            event_count: 0,
        }
    }
}

// ─── URL normalisation ────────────────────────────────────────────────────────

/// Normalise a feed URL: rewrite `webcal://` to `https://`, reject other
/// non-`http(s)` schemes.
///
/// `webcal://` is the conventional scheme calendars advertise for subscription;
/// it is just HTTPS underneath. Rejecting every other scheme keeps a feed from
/// ever pointing at a local file (`file://`) or an unexpected protocol — the
/// first line of the egress guard (the fetch layer enforces the rest).
///
/// # Errors
///
/// Returns [`CalendarError::InvalidUrl`] if the string does not start with a
/// recognised scheme.
pub fn normalise_feed_url(url: &str) -> Result<String, CalendarError> {
    // Trim incidental whitespace a user may paste around a URL.
    let trimmed = url.trim();
    // webcal:// is HTTPS by convention — rewrite it so the fetch layer sees https.
    if let Some(rest) = trimmed.strip_prefix("webcal://") {
        return Ok(format!("https://{rest}"));
    }
    // Only http and https are permitted to reach the network.
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_owned());
    }
    // Anything else (file://, ftp://, a bare host, empty) is rejected up front.
    Err(CalendarError::InvalidUrl(trimmed.to_owned()))
}

// ─── Builder ──────────────────────────────────────────────────────────────────

/// Validates invariants before constructing a [`Calendar`].
///
/// Enforces a non-empty name and a normalised `http(s)` URL, and requires an
/// injected `now`. `id` defaults to a fresh one but can be supplied (the storage
/// layer never uses the builder, but tests and the API handler do).
pub struct CalendarBuilder {
    /// The raw name; trimmed and checked at build time.
    /// User input is stored as-is until validation, where it is trimmed.
    /// Empty strings (after trimming) are rejected in the `build` step.
    name: String,
    /// The raw URL; normalised and checked at build time.
    /// Input URLs may use `webcal://` (rewritten to `https://`) or any
    /// `http(s)://` scheme; other schemes are rejected at build time.
    url: String,
    /// Advisory category; defaults to Other.
    /// Categories are user-facing hints for grouping and display — they do not
    /// affect feed fetching or parsing. `Other` is safe when uncertain.
    category: CalendarCategory,
    /// Enabled by default — a freshly added feed should sync.
    /// The sync job skips disabled feeds without updating their sync state.
    /// Users can disable feeds to keep them from fetching without deleting them.
    enabled: bool,
    /// Injected clock; required (no hidden clock reads in core).
    /// The creation timestamp is recorded for audit and sorting purposes.
    /// The builder requires this be set explicitly to avoid hidden time-dependence.
    now: Option<OffsetDateTime>,
    /// Optional explicit id; a fresh one is generated when absent.
    /// The storage layer does not use the builder and reconstructs `Calendar`
    /// directly from database rows, setting the id explicitly. Tests and the
    /// API handler may also supply a known id for reproducibility.
    id: Option<CalendarId>,
}

impl CalendarBuilder {
    /// Start a builder from the two required fields.
    ///
    /// The `name` and `url` are the only required parameters at construction.
    /// All other fields have sensible defaults (category=Other, enabled=true).
    /// The `now` timestamp MUST be set via `.now(...)` before calling `.build()`.
    /// The id defaults to a fresh one but can be overridden via `.id(...)` for
    /// testing or when reconstructing from storage.
    #[must_use]
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        // Sensible defaults; the required `now` is set via `.now(...)`.
        Self {
            name: name.into(),
            url: url.into(),
            category: CalendarCategory::Other,
            enabled: true,
            now: None,
            id: None,
        }
    }

    /// Set the advisory category (default `Other`).
    ///
    /// Used for grouping and display treatment in the UI. Does not affect
    /// how the feed is fetched or parsed. Useful for distinguishing school
    /// calendars from waste collections or personal calendars.
    #[must_use]
    pub fn category(mut self, category: CalendarCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the enabled flag (default `true`).
    ///
    /// Disabled calendars are not synced by the background job. Use this to
    /// archive a feed without deleting it. When re-enabled, the next sync will
    /// fetch the feed as normal.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Inject the creation clock (required).
    ///
    /// This timestamp becomes the `created_at` field in the final `Calendar`.
    /// Core never reads the system clock; it is always injected to make
    /// construction deterministic and testable. Callers are responsible for
    /// providing a trustworthy time, typically from the HTTP request context
    /// or the service layer.
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Supply an explicit id instead of generating a fresh one.
    ///
    /// By default, `.build()` generates a fresh UUID v7. This method lets
    /// callers override that — useful for tests that need reproducible ids,
    /// and for the storage layer to reconstruct a `Calendar` from a database
    /// row with its original id intact.
    #[must_use]
    pub fn id(mut self, id: CalendarId) -> Self {
        self.id = Some(id);
        self
    }

    /// Validate and construct the [`Calendar`].
    ///
    /// # Errors
    ///
    /// - [`CalendarError::EmptyName`] if the name is blank after trimming.
    /// - [`CalendarError::InvalidUrl`] if the URL scheme is not `http(s)`/`webcal`.
    /// - [`CalendarError::MissingNow`] if `now` was not injected.
    pub fn build(self) -> Result<Calendar, CalendarError> {
        // A blank name carries no information and would render as an empty row.
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err(CalendarError::EmptyName);
        }
        // Normalise + scheme-check the URL (webcal→https, reject the rest).
        let url = normalise_feed_url(&self.url)?;
        // The clock must be injected — core never reads it itself.
        let created_at = self.now.ok_or(CalendarError::MissingNow)?;
        // A fresh time-ordered id unless the caller supplied one.
        let id = self.id.unwrap_or_default();
        Ok(Calendar {
            id,
            name,
            url,
            category: self.category,
            enabled: self.enabled,
            created_at,
        })
    }
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Construction and parse failures for the Calendar entity.
#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    /// The feed name was blank.
    #[error("calendar name must not be empty")]
    EmptyName,
    /// The URL scheme was not http(s)/webcal.
    #[error("calendar url must be http(s): {0}")]
    InvalidUrl(String),
    /// The builder's `now` clock was not injected.
    #[error("required field not set: now")]
    MissingNow,
    /// A category storage string was not recognised.
    #[error("unknown calendar category: {0}")]
    UnknownCategory(String),
    /// A sync-status storage string was not recognised.
    #[error("unknown sync status: {0}")]
    UnknownSyncStatus(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn now() -> time::OffsetDateTime {
        datetime!(2026-07-26 12:00:00 UTC)
    }

    #[test]
    fn builds_a_valid_calendar() {
        let cal = CalendarBuilder::new("Zutphen afvalkalender", "https://example.test/waste.ics")
            .category(CalendarCategory::Waste)
            .now(now())
            .build()
            .expect("valid calendar builds");
        assert_eq!(cal.name, "Zutphen afvalkalender");
        assert_eq!(cal.category, CalendarCategory::Waste);
        assert!(cal.enabled, "calendars are enabled by default");
    }

    #[test]
    fn rejects_an_empty_name() {
        let err = CalendarBuilder::new("   ", "https://example.test/x.ics")
            .now(now())
            .build()
            .unwrap_err();
        assert!(matches!(err, CalendarError::EmptyName));
    }

    #[test]
    fn normalises_webcal_to_https() {
        let cal = CalendarBuilder::new("school", "webcal://example.test/s.ics")
            .now(now())
            .build()
            .expect("webcal is accepted");
        assert_eq!(cal.url, "https://example.test/s.ics");
    }

    #[test]
    fn rejects_a_non_http_scheme() {
        let err = CalendarBuilder::new("bad", "file:///etc/passwd")
            .now(now())
            .build()
            .unwrap_err();
        assert!(matches!(err, CalendarError::InvalidUrl(_)));
    }

    #[test]
    fn category_round_trips_through_its_string() {
        for c in [
            CalendarCategory::School,
            CalendarCategory::Club,
            CalendarCategory::Waste,
            CalendarCategory::Holiday,
            CalendarCategory::Personal,
            CalendarCategory::Other,
        ] {
            let s = c.to_string();
            assert_eq!(s.parse::<CalendarCategory>().unwrap(), c);
        }
    }

    #[test]
    fn sync_status_round_trips_through_its_string() {
        for s in [
            SyncStatus::Never,
            SyncStatus::Ok,
            SyncStatus::Unreachable,
            SyncStatus::ParseError,
        ] {
            assert_eq!(s.to_string().parse::<SyncStatus>().unwrap(), s);
        }
    }
}
