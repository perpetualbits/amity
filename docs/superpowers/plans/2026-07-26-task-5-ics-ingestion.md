# Task 5 — ICS Ingestion & External Calendar Aggregation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fetch read-only external ICS calendar feeds, parse them, and surface their events on Today alongside tasks and native events.

**Architecture:** Preserve the existing three-layer split. `amity-core` gets the pure, I/O-free parts (a `Calendar` entity + ICS parse/expand). `amity-storage` gets migration `0004`, a `calendars` repository, and idempotent external-event upsert/prune. `amity-service` gets the outbound fetch, a background sync job (mirroring `recurrence_horizon`), and the calendars API. Surfacing needs no change — ingested events ride Task 4's `build_event_candidates` onto Today.

**Tech Stack:** Rust workspace (`amity-core`/`amity-storage`/`amity-service`), sqlx + SQLite (STRICT), axum, tokio, `reqwest` (already a workspace dep, rustls), the `rrule` crate (already a dep), and one ICS parser crate (`ical`) added in Task 2.

**Design spec:** `docs/superpowers/specs/2026-07-26-task-5-ics-ingestion-design.md`

## Global Constraints

Every task's requirements implicitly include this section.

- **TDD, always.** Write the failing test, run it and watch it fail for the right reason, write the minimal code, watch it pass. No production code without a failing test first.
- **Comment density ≥ 50%** on every production `crates/**/*.rs` file. Gate: `find crates -name '*.rs' | xargs bash scripts/comment-density.sh`. The gate is string-literal-aware (SQL/ICS-fixture bodies do not count as code) and excludes test code (`tests/` files and `#[cfg(test)]` blocks). Write genuine explanatory comments, not padding.
- **fmt clean:** `cargo fmt --all -- --check`. **clippy clean:** `cargo clippy --workspace --all-targets -- -W clippy::pedantic`.
- **Tests green:** `cargo test --workspace`.
- **Commits:** Conventional Commits with DCO sign-off — `git commit -s`. Prefix `feat(task-5)` for code, `docs(task-5)` for docs. Never put backticks in a `-m` message from a shell (they run as command substitution); use `-F <file>` for multi-line/backtick messages.
- **Clock injection:** `amity-core` never reads the clock; callers pass `now: OffsetDateTime`. Builders take an injected `now`.
- **Storage conventions:** RFC 3339 TEXT datetimes, TEXT UUIDs, INTEGER 0/1 booleans, STRICT tables. Stored enums use `Display`/`FromStr` for snake_case strings. `thiserror` for error enums.
- **`amity-core` stays I/O-free:** no tokio, no sqlx, no reqwest. The `ical` and `rrule` crates parse strings — pure, allowed.
- **`apps/hub-tauri` is outside the workspace** (needs WebKit2GTK, absent here). It builds only with `vite build` / cannot be run live. Task 7 is optional and type-checked only.
- **Horizon:** reuse the 60-day forward horizon used by tasks and native events.

---

## File Structure

**Task 1 (core — `Calendar` entity):**
- Modify `crates/amity-core/src/ids.rs` — add `CalendarId`.
- Create `crates/amity-core/src/calendar.rs` — `Calendar`, `CalendarBuilder`, `CalendarCategory`, `SyncStatus`, `CalendarSyncState`, `CalendarError`.
- Modify `crates/amity-core/src/lib.rs` — `pub mod calendar;`.

**Task 2 (core — ICS parse/expand):**
- Create `crates/amity-core/src/ics.rs` — `ParsedEvent`, `parse_feed`, `expand_external`, `IcsError`.
- Modify `crates/amity-core/src/lib.rs` — `pub mod ics;`.
- Modify `Cargo.toml` (root) + `crates/amity-core/Cargo.toml` — add the `ical` parser crate.

**Task 3 (storage — migration + repo + upsert/prune):**
- Create `crates/amity-storage/migrations/0004_add_calendars.sql`.
- Create `crates/amity-storage/src/calendar.rs` — calendars repository.
- Modify `crates/amity-storage/src/event.rs` — add `upsert_external_events`, `prune_events_missing_from_feed`.
- Modify `crates/amity-storage/src/lib.rs` — `pub mod calendar;`.
- Create `crates/amity-storage/tests/calendar_repository.rs`.

**Task 4 (service — fetch + sync job):**
- Create `crates/amity-service/src/feeds.rs` — `fetch`, `FetchError`.
- Create `crates/amity-service/src/jobs/calendar_sync.rs` — `run_once`, `sync_one`, `spawn`, `SyncReport`.
- Modify `crates/amity-service/src/jobs/mod.rs` — `pub mod calendar_sync;`.
- Modify `crates/amity-service/src/lib.rs` — `pub mod feeds;`.
- Modify `crates/amity-service/src/main.rs` — spawn the job.
- Create `crates/amity-service/tests/calendar_sync.rs`.

**Task 5 (service — API + e2e):**
- Create `crates/amity-service/src/api/calendar.rs` — handlers.
- Modify `crates/amity-service/src/api/mod.rs` — `pub mod calendar;`.
- Modify `crates/amity-service/src/lib.rs` — routes.
- Create `crates/amity-service/tests/calendar_api.rs`.

**Task 6 (docs):**
- Create `docs/adrs/0004-external-calendar-ingestion.md`.
- Modify `project-map.js`.

**Task 7 (optional — hub Calendars list):**
- Modify `apps/hub-tauri/src/api.ts`, add `apps/hub-tauri/src/Calendars.tsx`, wire into `App.tsx`.

---

## Task 1: `Calendar` domain entity (core)

**Files:**
- Modify: `crates/amity-core/src/ids.rs` (add `CalendarId` near the other `define_id!` calls)
- Create: `crates/amity-core/src/calendar.rs`
- Modify: `crates/amity-core/src/lib.rs` (add `pub mod calendar;` in alphabetical position)
- Test: `#[cfg(test)] mod tests` inside `calendar.rs`

**Interfaces:**
- Consumes: `define_id!` macro (ids.rs), `time::OffsetDateTime`, `serde`.
- Produces:
  - `CalendarId` — UUID-v7 newtype (Display/FromStr/serde via the macro).
  - `enum CalendarCategory { School, Club, Waste, Holiday, Personal, Other }` with `Display`/`FromStr` (snake_case) and `Default = Other`.
  - `enum SyncStatus { Never, Ok, Unreachable, ParseError }` with `Display`/`FromStr` (snake_case) and `Default = Never`.
  - `struct Calendar { id: CalendarId, name: String, url: String, category: CalendarCategory, enabled: bool, created_at: OffsetDateTime }` (fields `pub` so storage can reconstruct).
  - `struct CalendarSyncState { last_synced_at: Option<OffsetDateTime>, last_status: SyncStatus, last_error: Option<String>, event_count: u32 }`.
  - `CalendarBuilder` with `new(name, url) -> Self`, `.category(CalendarCategory)`, `.enabled(bool)`, `.now(OffsetDateTime)`, `.id(CalendarId)`, `.build() -> Result<Calendar, CalendarError>`.
  - `enum CalendarError { EmptyName, InvalidUrl(String), MissingNow, UnknownCategory(String), UnknownSyncStatus(String) }` (thiserror).
  - Free fn `normalise_feed_url(&str) -> Result<String, CalendarError>` — rewrites `webcal://` → `https://`, rejects non-`http(s)` schemes.

- [ ] **Step 1: Add `CalendarId` to ids.rs**

In `crates/amity-core/src/ids.rs`, add alongside the other `define_id!` invocations:

```rust
define_id!(
    /// Unique identifier for a [`Calendar`](crate::calendar::Calendar) — one
    /// subscribed external ICS feed. See Task 5 and brief §7.
    CalendarId
);
```

- [ ] **Step 2: Register the module**

In `crates/amity-core/src/lib.rs`, add (alphabetical order — before `completion_log`):

```rust
pub mod calendar;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/amity-core/src/calendar.rs` with only the test module first (so it fails to compile → the RED state):

```rust
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
        for s in [SyncStatus::Never, SyncStatus::Ok, SyncStatus::Unreachable, SyncStatus::ParseError] {
            assert_eq!(s.to_string().parse::<SyncStatus>().unwrap(), s);
        }
    }
}
```

- [ ] **Step 4: Run the tests, verify they fail to compile**

Run: `cargo test -p amity-core calendar`
Expected: FAIL — `CalendarBuilder`, `Calendar`, etc. are undefined.

- [ ] **Step 5: Implement the module**

Prepend the implementation above the test module in `crates/amity-core/src/calendar.rs`. Match the enum `Display`/`FromStr` pattern from `event.rs` (`EventSourceKind`) exactly, and the builder shape from `EventBuilder`. Every non-blank line needs its share of explanatory comments (density gate).

```rust
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
    /// Stable identifier.
    pub id: CalendarId,
    /// Human-facing display name (non-empty, trimmed).
    pub name: String,
    /// The feed URL — always `http`/`https` after normalisation.
    pub url: String,
    /// Advisory grouping category.
    pub category: CalendarCategory,
    /// Whether the sync job fetches this feed. Disabled feeds are skipped.
    pub enabled: bool,
    /// When the subscription was created.
    pub created_at: OffsetDateTime,
}

/// The runtime sync health of a feed, updated by the sync job.
///
/// Kept out of [`Calendar`] so the entity is a stable description, not a
/// mutable status record. The repository returns both together in its read
/// model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarSyncState {
    /// When the last successful sync completed; `None` until the first success.
    pub last_synced_at: Option<OffsetDateTime>,
    /// The outcome of the most recent attempt.
    pub last_status: SyncStatus,
    /// A short diagnostic when the last attempt failed; `None` otherwise.
    pub last_error: Option<String>,
    /// How many events the last good sync produced.
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
    // The raw name; trimmed and checked at build time.
    name: String,
    // The raw URL; normalised and checked at build time.
    url: String,
    // Advisory category; defaults to Other.
    category: CalendarCategory,
    // Enabled by default — a freshly added feed should sync.
    enabled: bool,
    // Injected clock; required (no hidden clock reads in core).
    now: Option<OffsetDateTime>,
    // Optional explicit id; a fresh one is generated when absent.
    id: Option<CalendarId>,
}

impl CalendarBuilder {
    /// Start a builder from the two required fields.
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
    #[must_use]
    pub fn category(mut self, category: CalendarCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the enabled flag (default `true`).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Inject the creation clock (required).
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Supply an explicit id instead of generating a fresh one.
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
```

Confirm `thiserror` is already a dependency of `amity-core` (it is — `EventError` uses it).

- [ ] **Step 6: Run tests, verify pass + gate**

Run: `cargo test -p amity-core calendar`
Expected: PASS (all six tests).
Then: `cargo fmt -p amity-core && cargo clippy -p amity-core --all-targets -- -W clippy::pedantic` and `find crates/amity-core -name '*.rs' | xargs bash scripts/comment-density.sh` — all clean, `calendar.rs` ≥ 50%.

- [ ] **Step 7: Commit**

```bash
git add crates/amity-core/src/calendar.rs crates/amity-core/src/ids.rs crates/amity-core/src/lib.rs
git commit -s -m "feat(task-5): add Calendar domain type, category, and sync status"
```

---

## Task 2: ICS parsing & external-recurrence expansion (core, pure)

**Files:**
- Modify: `Cargo.toml` (root) — add `ical` to `[workspace.dependencies]`.
- Modify: `crates/amity-core/Cargo.toml` — depend on `ical`.
- Create: `crates/amity-core/src/ics.rs`.
- Modify: `crates/amity-core/src/lib.rs` — `pub mod ics;`.
- Test: `#[cfg(test)] mod tests` inside `ics.rs` (fixtures as string consts).

**Interfaces:**
- Consumes: the `ical` crate (parser), the `rrule` crate (expansion — mirror `recurrence_materialiser.rs`), `time::OffsetDateTime`.
- Produces:
  - `struct ParsedEvent { uid: String, summary: String, start: OffsetDateTime, end: Option<OffsetDateTime>, all_day: bool, rrule: Option<String>, exdates: Vec<OffsetDateTime>, tzid: Option<String> }`.
  - `fn parse_feed(text: &str) -> Result<Vec<ParsedEvent>, IcsError>`.
  - `fn expand_external(event: &ParsedEvent, from: OffsetDateTime, to: OffsetDateTime) -> Vec<OffsetDateTime>`.
  - `enum IcsError { NotCalendar }` (thiserror).

**Notes for the implementer:** the concrete parser crate is an internal detail hidden behind `parse_feed`. This plan uses the `ical` crate (`IcalParser` over a byte reader, each `IcalEvent` exposing a `properties: Vec<Property>` bag). Extract properties by name (`SUMMARY`, `UID`, `DTSTART`, `DTEND`, `RRULE`, `EXDATE`), reading `Property.value` and `Property.params`. If an accessor name differs in the crate version you resolve, adjust it — the tests below (against *our* `ParsedEvent`) are the contract, not the crate's API shape. Datetime parsing handles two ICS forms: `YYYYMMDDTHHMMSS[Z]` (timed; `Z`=UTC, else interpret in `TZID` or default `Europe/Amsterdam`) and `YYYYMMDD` (all-day; `VALUE=DATE` param present → `all_day = true`, time set to local midnight).

- [ ] **Step 1: Add the parser dependency**

In root `Cargo.toml` under `[workspace.dependencies]`:

```toml
# iCalendar (RFC 5545) parser for read-only external feed ingestion (Task 5).
# Used only inside amity-core's ics module, behind our own parse_feed wrapper.
ical = { version = "0.11", default-features = false, features = ["ical"] }
```

In `crates/amity-core/Cargo.toml` under `[dependencies]`:

```toml
# ICS parsing for external calendar feeds (pure text→struct, no I/O).
ical = { workspace = true }
```

- [ ] **Step 2: Register the module**

In `crates/amity-core/src/lib.rs` add (alphabetical — after `ids`):

```rust
pub mod ics;
```

- [ ] **Step 3: Write the failing tests (with fixtures)**

Create `crates/amity-core/src/ics.rs` with the test module first. The fixtures are `const &str` — the density gate does not count string-literal body lines, so they cost nothing against the ratio.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    // A minimal single timed VEVENT.
    const SINGLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:abc-1\r\nSUMMARY:Dentist\r\nDTSTART:20990202T090000Z\r\nDTEND:20990202T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    // An all-day VEVENT (VALUE=DATE).
    const ALL_DAY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:kd-1\r\nSUMMARY:King's Day\r\nDTSTART;VALUE=DATE:20990427\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    // A recurring VEVENT with a weekly rule.
    const RECURRING: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:gym-1\r\nSUMMARY:Gym\r\nDTSTART:20990106T180000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    // A good event followed by a broken one (missing DTSTART).
    const ONE_BAD: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:ok-1\r\nSUMMARY:Good\r\nDTSTART:20990202T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:bad-1\r\nSUMMARY:No start\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parses_a_single_timed_event() {
        let events = parse_feed(SINGLE).expect("valid feed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "abc-1");
        assert_eq!(events[0].summary, "Dentist");
        assert!(!events[0].all_day);
        assert_eq!(events[0].start, datetime!(2099-02-02 09:00:00 UTC));
    }

    #[test]
    fn parses_an_all_day_event() {
        let events = parse_feed(ALL_DAY).expect("valid feed");
        assert_eq!(events.len(), 1);
        assert!(events[0].all_day);
    }

    #[test]
    fn a_malformed_vevent_is_skipped_not_fatal() {
        let events = parse_feed(ONE_BAD).expect("feed still parses");
        assert_eq!(events.len(), 1, "the good event survives, the bad one is dropped");
        assert_eq!(events[0].uid, "ok-1");
    }

    #[test]
    fn a_non_calendar_payload_is_an_error() {
        let err = parse_feed("this is not a calendar").unwrap_err();
        assert!(matches!(err, IcsError::NotCalendar));
    }

    #[test]
    fn expands_a_weekly_rule_within_a_window() {
        let events = parse_feed(RECURRING).expect("valid feed");
        let from = datetime!(2099-01-01 00:00:00 UTC);
        let to = datetime!(2099-02-01 00:00:00 UTC);
        let instants = expand_external(&events[0], from, to);
        // Mondays in Jan 2099 from the 6th: 6, 13, 20, 27 → 4 instances.
        assert_eq!(instants.len(), 4);
    }

    #[test]
    fn a_non_recurring_event_expands_to_its_single_start() {
        let events = parse_feed(SINGLE).expect("valid feed");
        let from = datetime!(2099-01-01 00:00:00 UTC);
        let to = datetime!(2099-12-31 00:00:00 UTC);
        let instants = expand_external(&events[0], from, to);
        assert_eq!(instants, vec![datetime!(2099-02-02 09:00:00 UTC)]);
    }
}
```

- [ ] **Step 4: Run the tests, verify they fail**

Run: `cargo test -p amity-core ics`
Expected: FAIL to compile — `parse_feed`, `ParsedEvent`, etc. undefined.

- [ ] **Step 5: Implement `parse_feed`, `expand_external`, `ParsedEvent`, `IcsError`**

Write the implementation above the test module. Key logic (comment every line for the density gate):

- `parse_feed`: reject a payload with no `BEGIN:VCALENDAR` as `IcsError::NotCalendar`. Feed the text to `ical::IcalParser::new(text.as_bytes())`. For each parsed calendar, iterate its events; map each through a fallible `parse_one(&IcalEvent) -> Option<ParsedEvent>` that returns `None` (logged via `tracing::debug!`) when a required field (`UID`, `SUMMARY`, `DTSTART`) is missing or unparseable. Collect the `Some` values. An empty result is valid.
- Datetime parsing helper `parse_ics_datetime(value, params) -> Option<(OffsetDateTime, bool)>` returning `(instant, all_day)`: if a `VALUE=DATE` param is present, parse `YYYYMMDD` at local midnight → `all_day = true`; else parse `YYYYMMDDTHHMMSS` with optional trailing `Z`. For `Z`, offset is UTC; otherwise apply the `TZID` param via the same `Europe/Amsterdam` default the rest of the codebase uses. Use `time`'s parsing; keep the helper pure.
- `EXDATE`: parse each comma/line value through the same helper, collect into `exdates`.
- `expand_external`: if `event.rrule` is `None`, return `vec![event.start]` when `from <= event.start <= to`, else empty. If `Some(rrule)`, build the `DTSTART;TZID=…:…\nRRULE:…` block exactly as `recurrence_materialiser::materialise_instances` does (mirror that code — build the DTSTART from the event's local-time components and `tzid`/default zone), parse it as `rrule::RRuleSet`, iterate instances, keep those within `[from, to]` and not in `exdates`, convert `chrono::DateTime` → `time::OffsetDateTime` the same way the materialiser does. Apply the same 1000-instance safety cap.

```rust
// ics.rs — pure iCalendar (RFC 5545) parsing for read-only external feeds.
//
// This module turns raw ICS feed text into a Vec of `ParsedEvent`, and expands
// a recurring parsed event into concrete instants within a window. It performs
// NO I/O — the service layer fetches the bytes; this module only parses the
// string it is handed, which keeps the whole parser unit-testable against
// fixture feeds with no network.
//
// Scope (brief §7, the Task 5 spec): SUMMARY, DTSTART/DTEND, all-day
// (VALUE=DATE), TZID, RRULE, EXDATE, UID. VALARM, RECURRENCE-ID, RDATE, and
// attendee/organizer fields are ignored. A malformed VEVENT is skipped, not
// fatal — one bad event never sinks a whole feed. `IcsError` is reserved for a
// payload that is not iCalendar at all.
//
// Recurrence expansion uses the FULL rrule crate directly, NOT the native
// recurrence validator in recurrence.rs. That validator deliberately rejects
// BYSETPOS/BYWEEKNO/sub-daily FREQ to keep native recurrence simple, but
// external feeds are read-only and must render whatever real calendars emit.

// ... (ParsedEvent, IcsError, parse_feed, parse_one, parse_ics_datetime,
//      expand_external — implemented per the logic notes above) ...
```

*(The implementer writes the bodies; the tests in Step 3 are the acceptance contract. Mirror `recurrence_materialiser.rs:74-140` for the rrule integration and the chrono→time conversion.)*

- [ ] **Step 6: Run the tests, verify pass**

Run: `cargo test -p amity-core ics`
Expected: PASS (all six).
Then fmt + clippy pedantic + density gate on `crates/amity-core`, all clean.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/amity-core/Cargo.toml crates/amity-core/src/ics.rs crates/amity-core/src/lib.rs
git commit -s -m "feat(task-5): add pure ICS parsing and external-recurrence expansion"
```

---

## Task 3: Storage — migration `0004`, calendars repo, external-event upsert/prune

**Files:**
- Create: `crates/amity-storage/migrations/0004_add_calendars.sql`
- Create: `crates/amity-storage/src/calendar.rs`
- Modify: `crates/amity-storage/src/event.rs` — add `upsert_external_events`, `prune_events_missing_from_feed`
- Modify: `crates/amity-storage/src/lib.rs` — `pub mod calendar;`
- Test: `crates/amity-storage/tests/calendar_repository.rs`

**Interfaces:**
- Consumes: `amity_core::calendar::{Calendar, CalendarCategory, CalendarSyncState, SyncStatus, CalendarId}`, `amity_core::event::Event`, `crate::StorageError`, `crate::event_instance::upsert_event_instances`.
- Produces (in `calendar.rs`):
  - `struct StoredCalendar { calendar: Calendar, sync: CalendarSyncState }` — the read model.
  - `async fn insert_calendar(pool, &Calendar) -> Result<(), StorageError>`.
  - `async fn list_calendars(pool) -> Result<Vec<StoredCalendar>, StorageError>`.
  - `async fn fetch_calendar(pool, CalendarId) -> Result<Option<StoredCalendar>, StorageError>`.
  - `async fn set_calendar_enabled(pool, CalendarId, bool) -> Result<bool, StorageError>` (returns whether a row matched).
  - `async fn delete_calendar(pool, CalendarId) -> Result<bool, StorageError>` (cascades to that calendar's events + instances).
  - `async fn update_calendar_sync_state(pool, CalendarId, &CalendarSyncState) -> Result<(), StorageError>`.
- Produces (in `event.rs`):
  - `async fn upsert_external_events(pool, &[Event]) -> Result<(), StorageError>` — insert-or-update keyed on `(source_calendar_id, source_external_id)`.
  - `async fn prune_events_missing_from_feed(pool, calendar_id: &str, keep_external_ids: &[String]) -> Result<u64, StorageError>` — delete this calendar's events (and their instances) whose UID is not in `keep_external_ids`; returns rows deleted.

- [ ] **Step 1: Write migration `0004_add_calendars.sql`**

```sql
-- 0004_add_calendars.sql — external calendar subscriptions (Task 5).
--
-- A `calendars` row is one subscribed read-only ICS feed. Its events live in the
-- existing `events` table (migration 0003), linked by `source_calendar_id` and
-- identified within the feed by `source_external_id` (the VEVENT UID). A partial
-- UNIQUE index on that pair makes re-sync an idempotent upsert; it is partial so
-- native events (NULL source_calendar_id) are never constrained.

CREATE TABLE calendars (
    id              TEXT    NOT NULL PRIMARY KEY,   -- UUID v7
    name            TEXT    NOT NULL,               -- display name (non-empty)
    url             TEXT    NOT NULL,               -- http(s) feed URL
    category        TEXT    NOT NULL,               -- snake_case CalendarCategory
    enabled         INTEGER NOT NULL,               -- 0/1
    created_at      TEXT    NOT NULL,               -- RFC 3339
    last_synced_at  TEXT,                           -- RFC 3339, NULL until first success
    last_status     TEXT    NOT NULL,               -- snake_case SyncStatus
    last_error      TEXT,                           -- short diagnostic on failure
    event_count     INTEGER NOT NULL                -- events from the last good sync
) STRICT;

-- Idempotent upsert key for a feed's events. Partial so only external events
-- (with a non-NULL calendar id) participate; native events are unaffected.
CREATE UNIQUE INDEX idx_events_source_unique
    ON events (source_calendar_id, source_external_id)
    WHERE source_calendar_id IS NOT NULL;
```

- [ ] **Step 2: Register the module**

In `crates/amity-storage/src/lib.rs` add `pub mod calendar;` in alphabetical position.

- [ ] **Step 3: Write the failing repository tests**

Create `crates/amity-storage/tests/calendar_repository.rs`. Mirror the harness in `tests/event_repository.rs` (open `sqlite::memory:` via `open_database`). Cover: insert→list round-trip with default sync state; `set_calendar_enabled` toggles; `update_calendar_sync_state` persists status+error+count; `delete_calendar` removes the row; `upsert_external_events` inserts then UPDATEs a changed title for the same `(calendar_id, uid)`; `prune_events_missing_from_feed` deletes the vanished UID and returns 1.

```rust
// calendar_repository.rs — integration tests for the calendars repository and
// the external-event upsert/prune helpers, over a fresh in-memory database.

use amity_core::calendar::{CalendarBuilder, CalendarCategory, CalendarSyncState, SyncStatus};
use amity_core::event::{EventBuilder, EventSource};
use amity_storage::calendar::{
    delete_calendar, fetch_calendar, insert_calendar, list_calendars, set_calendar_enabled,
    update_calendar_sync_state,
};
use amity_storage::connection::open_database;
use amity_storage::event::{prune_events_missing_from_feed, upsert_external_events};
use time::macros::datetime;

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-07-26 12:00:00 UTC)
}

#[tokio::test]
async fn insert_then_list_round_trips_with_default_sync_state() {
    let pool = db().await;
    let cal = CalendarBuilder::new("school", "https://example.test/s.ics")
        .category(CalendarCategory::School)
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    let all = list_calendars(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].calendar.name, "school");
    assert_eq!(all[0].sync.last_status, SyncStatus::Never);
    assert_eq!(all[0].sync.event_count, 0);
}

#[tokio::test]
async fn update_sync_state_persists() {
    let pool = db().await;
    let cal = CalendarBuilder::new("holidays", "https://example.test/h.ics")
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    let state = CalendarSyncState {
        last_synced_at: Some(now()),
        last_status: SyncStatus::Ok,
        last_error: None,
        event_count: 12,
    };
    update_calendar_sync_state(&pool, cal.id, &state).await.unwrap();

    let fetched = fetch_calendar(&pool, cal.id).await.unwrap().unwrap();
    assert_eq!(fetched.sync.last_status, SyncStatus::Ok);
    assert_eq!(fetched.sync.event_count, 12);
}

#[tokio::test]
async fn upsert_external_events_inserts_then_updates() {
    let pool = db().await;
    // Two builds with the SAME source (calendar_id + uid) but different titles.
    let make = |title: &str| {
        EventBuilder::new()
            .title(title)
            .start_at(datetime!(2099-05-01 09:00:00 UTC))
            .source(EventSource::ics("uid-1", "cal-1", now()))
            .now(now())
            .build()
            .unwrap()
    };
    upsert_external_events(&pool, &[make("First")]).await.unwrap();
    upsert_external_events(&pool, &[make("Second")]).await.unwrap();

    // Exactly one row for that source, carrying the updated title.
    let events = amity_storage::event::list_events(&pool).await.unwrap();
    let mine: Vec<_> = events.iter().filter(|e| e.source.external_id.as_deref() == Some("uid-1")).collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].title, "Second");
}

#[tokio::test]
async fn prune_removes_events_missing_from_the_feed() {
    let pool = db().await;
    let make = |uid: &str| {
        EventBuilder::new()
            .title(uid)
            .start_at(datetime!(2099-05-01 09:00:00 UTC))
            .source(EventSource::ics(uid, "cal-1", now()))
            .now(now())
            .build()
            .unwrap()
    };
    upsert_external_events(&pool, &[make("keep"), make("drop")]).await.unwrap();

    // Re-sync sees only "keep"; "drop" must be pruned.
    let deleted = prune_events_missing_from_feed(&pool, "cal-1", &["keep".to_owned()])
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let events = amity_storage::event::list_events(&pool).await.unwrap();
    assert!(events.iter().all(|e| e.source.external_id.as_deref() != Some("drop")));
}

#[tokio::test]
async fn set_enabled_and_delete_work() {
    let pool = db().await;
    let cal = CalendarBuilder::new("club", "https://example.test/c.ics")
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    assert!(set_calendar_enabled(&pool, cal.id, false).await.unwrap());
    assert!(!fetch_calendar(&pool, cal.id).await.unwrap().unwrap().calendar.enabled);

    assert!(delete_calendar(&pool, cal.id).await.unwrap());
    assert!(fetch_calendar(&pool, cal.id).await.unwrap().is_none());
}
```

Confirm `EventBuilder` exposes `.source(EventSource)` and `.start_at(...)`; check `crates/amity-core/src/event.rs` builder methods and adjust the calls to the actual method names before running.

- [ ] **Step 4: Run tests, verify they fail**

Run: `cargo test -p amity-storage --test calendar_repository`
Expected: FAIL — the repository functions and helpers are undefined.

- [ ] **Step 5: Implement `calendar.rs` repository**

Mirror `crates/amity-storage/src/event.rs` structure: a `CalendarRow` `#[derive(sqlx::FromRow)]`, a `CALENDAR_SELECT` column-list const, `row_to_stored` conversion (parse category + status via `FromStr`, datetimes via `Rfc3339`, `enabled`/`event_count` from `i64`), then the functions. Use `format!("{CALENDAR_SELECT} WHERE id = ?")` etc. (never `concat!` — it needs a literal). `insert_calendar` binds all 11 columns with `last_status = "never"`, `event_count = 0`, `last_synced_at = NULL`. `delete_calendar` first deletes dependent `event_instances` and `events` for that calendar (SQLite has no cascade under STRICT unless declared), then the `calendars` row. Return `rows_affected() > 0` for the boolean functions.

- [ ] **Step 6: Implement the upsert/prune helpers in `event.rs`**

`upsert_external_events`: for each event, an `INSERT INTO events (...) VALUES (...) ON CONFLICT (source_calendar_id, source_external_id) DO UPDATE SET title = excluded.title, start_at = excluded.start_at, end_at = excluded.end_at, all_day = excluded.all_day, timezone = excluded.timezone, location = excluded.location, recurrence_rrule = excluded.recurrence_rrule, recurrence_timezone = excluded.recurrence_timezone, source_last_synced_at = excluded.source_last_synced_at, updated_at = excluded.updated_at`. Reuse the existing field-serialisation from `insert_event` (extract a shared `bind_event_columns` helper if it reduces duplication, or repeat the bind list). Run all upserts in a single transaction. `prune_events_missing_from_feed`: `DELETE FROM event_instances WHERE event_id IN (SELECT id FROM events WHERE source_calendar_id = ? AND source_external_id NOT IN (<placeholders>))` then `DELETE FROM events WHERE source_calendar_id = ? AND source_external_id NOT IN (<placeholders>)`; build the `NOT IN` placeholder list dynamically (bind each kept UID). When `keep_external_ids` is empty, delete all of that calendar's events. Return the events `rows_affected()`.

- [ ] **Step 7: Run tests, verify pass + gate**

Run: `cargo test -p amity-storage --test calendar_repository`
Expected: PASS. Then fmt + clippy pedantic + density gate on `crates/amity-storage`.

- [ ] **Step 8: Commit**

```bash
git add crates/amity-storage/migrations/0004_add_calendars.sql crates/amity-storage/src/calendar.rs crates/amity-storage/src/event.rs crates/amity-storage/src/lib.rs crates/amity-storage/tests/calendar_repository.rs
git commit -s -m "feat(task-5): add calendars storage, migration 0004, and external-event upsert/prune"
```

---

## Task 4: Service — outbound fetch + calendar sync job

**Files:**
- Create: `crates/amity-service/src/feeds.rs`
- Create: `crates/amity-service/src/jobs/calendar_sync.rs`
- Modify: `crates/amity-service/src/jobs/mod.rs` — `pub mod calendar_sync;`
- Modify: `crates/amity-service/src/lib.rs` — `pub mod feeds;`
- Modify: `crates/amity-service/src/main.rs` — spawn the job
- Test: `crates/amity-service/tests/calendar_sync.rs`

**Interfaces:**
- Consumes: `reqwest`, `amity_core::ics::{parse_feed, expand_external}`, `amity_core::event::{EventBuilder, EventSource}`, `amity_storage::calendar::*`, `amity_storage::event::{upsert_external_events, prune_events_missing_from_feed}`, `amity_storage::event_instance::upsert_event_instances`.
- Produces:
  - `feeds`: `async fn fetch(url: String) -> Result<String, FetchError>`; `enum FetchError { Request(String), TooLarge, BadStatus(u16) }`.
  - `calendar_sync`: `struct SyncReport { calendars_synced: usize, events_upserted: usize, events_pruned: u64 }`; `async fn sync_one<F, Fut>(pool, now, calendar: &StoredCalendar, fetch: &F) -> Result<usize, String>` where `F: Fn(String) -> Fut, Fut: Future<Output = Result<String, FetchError>>`; `async fn run_once<F, Fut>(pool, now, fetch: F) -> Result<SyncReport, String>` (same bounds); `fn spawn(pool)`.

- [ ] **Step 1: Write the failing sync test (network-free)**

Create `crates/amity-service/tests/calendar_sync.rs`. It injects a fetch closure returning fixture ICS, runs `run_once`, and asserts events land; then re-syncs with an event removed and asserts prune; then a failing fetch records `Unreachable` and keeps prior events.

```rust
// calendar_sync.rs — integration tests for the sync job, with an injected fetch
// closure so no network is touched.

use amity_core::calendar::{CalendarBuilder, SyncStatus};
use amity_service::jobs::calendar_sync::run_once;
use amity_service::feeds::FetchError;
use amity_storage::calendar::{fetch_calendar, insert_calendar, list_calendars};
use amity_storage::connection::open_database;
use amity_storage::event::list_events;
use std::future::ready;
use time::macros::datetime;

const TWO: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20990501T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:B\r\nDTSTART:20990502T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const ONE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20990501T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn now() -> time::OffsetDateTime {
    datetime!(2099-04-27 12:00:00 UTC)
}

async fn seeded_pool() -> (sqlx::SqlitePool, amity_core::ids::CalendarId) {
    let pool = open_database("sqlite::memory:").await.unwrap();
    let cal = CalendarBuilder::new("feed", "https://example.test/f.ics").now(now()).build().unwrap();
    let id = cal.id;
    insert_calendar(&pool, &cal).await.unwrap();
    (pool, id)
}

#[tokio::test]
async fn sync_ingests_then_prunes_and_records_ok() {
    let (pool, _id) = seeded_pool().await;

    run_once(&pool, now(), |_url| ready(Ok::<_, FetchError>(TWO.to_owned()))).await.unwrap();
    assert_eq!(list_events(&pool).await.unwrap().len(), 2);

    // Re-sync with B removed from the feed.
    run_once(&pool, now(), |_url| ready(Ok::<_, FetchError>(ONE.to_owned()))).await.unwrap();
    let events = list_events(&pool).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source.external_id.as_deref(), Some("a"));

    let cal = &list_calendars(&pool).await.unwrap()[0];
    assert_eq!(cal.sync.last_status, SyncStatus::Ok);
    assert_eq!(cal.sync.event_count, 1);
}

#[tokio::test]
async fn a_failed_fetch_records_unreachable_and_keeps_events() {
    let (pool, id) = seeded_pool().await;
    run_once(&pool, now(), |_url| ready(Ok::<_, FetchError>(TWO.to_owned()))).await.unwrap();

    // Next sync fails to fetch.
    run_once(&pool, now(), |_url| ready(Err::<String, _>(FetchError::BadStatus(503)))).await.unwrap();

    // Events are still there; status reflects the failure.
    assert_eq!(list_events(&pool).await.unwrap().len(), 2);
    let cal = fetch_calendar(&pool, id).await.unwrap().unwrap();
    assert_eq!(cal.sync.last_status, SyncStatus::Unreachable);
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p amity-service --test calendar_sync`
Expected: FAIL — `run_once`, `feeds::FetchError` undefined.

- [ ] **Step 3: Implement `feeds::fetch`**

`feeds.rs`: build a `reqwest::Client` with a 20s timeout and a bounded redirect policy (`reqwest::redirect::Policy::limited(5)`). `GET` the URL; map a non-2xx to `FetchError::BadStatus(status)`; stream the body with a running byte count, aborting past 5 MiB (`FetchError::TooLarge`); return the body `String`. Network/timeout errors → `FetchError::Request(e.to_string())`. Comment each guard.

- [ ] **Step 4: Implement `jobs::calendar_sync`**

Mirror `recurrence_horizon.rs`. `run_once(pool, now, fetch)` loads `list_calendars`, filters `enabled`, and for each calls `sync_one`, accumulating a `SyncReport`; a per-calendar error is logged and recorded (never aborts the loop). `sync_one`:
1. `fetch(calendar.url.clone()).await` — on `Err`, `update_calendar_sync_state(Unreachable, err, keep old count, keep old last_synced_at)` and return `Ok(0)` (keep existing events).
2. `parse_feed(&text)` — on `Err`, record `ParseError` and return `Ok(0)`.
3. Map each `ParsedEvent` to an `Event` via `EventBuilder` with `EventSource::ics(uid, calendar_id, now)`, all-day/rrule/times from the parsed event.
4. `upsert_external_events(pool, &events)`.
5. `prune_events_missing_from_feed(pool, calendar_id, &uids)`.
6. For each event: `expand_external(parsed, now, now + 60d)` → `EventInstance` rows → `upsert_event_instances`.
7. `update_calendar_sync_state(Ok, None, count = events.len(), last_synced_at = Some(now))`.

`spawn(pool)`: identical shape to `recurrence_horizon::spawn` but on a ~6h interval (`const RUN_INTERVAL_SECS: u64 = 6 * 60 * 60;`), calling `run_once(&pool, now, |url| feeds::fetch(url))`.

- [ ] **Step 5: Register modules + spawn in main**

`jobs/mod.rs`: `pub mod calendar_sync;`. `lib.rs`: `pub mod feeds;`. `main.rs`: after the existing `recurrence_horizon::spawn(db.clone());`, add `amity_service::jobs::calendar_sync::spawn(db.clone());`.

- [ ] **Step 6: Run tests, verify pass + gate**

Run: `cargo test -p amity-service --test calendar_sync`
Expected: PASS. Then fmt + clippy pedantic + density gate on `crates/amity-service`, and `cargo test --workspace` green.

- [ ] **Step 7: Commit**

```bash
git add crates/amity-service/src/feeds.rs crates/amity-service/src/jobs/calendar_sync.rs crates/amity-service/src/jobs/mod.rs crates/amity-service/src/lib.rs crates/amity-service/src/main.rs crates/amity-service/tests/calendar_sync.rs
git commit -s -m "feat(task-5): add ICS fetch and the calendar sync job"
```

---

## Task 5: Service — calendars API + end-to-end surfacing test

**Files:**
- Create: `crates/amity-service/src/api/calendar.rs`
- Modify: `crates/amity-service/src/api/mod.rs` — `pub mod calendar;`
- Modify: `crates/amity-service/src/lib.rs` — routes
- Test: `crates/amity-service/tests/calendar_api.rs`

**Interfaces:**
- Consumes: `axum` extractors, `AppState`, `amity_core::calendar::*`, `amity_storage::calendar::*`, `jobs::calendar_sync::sync_one`, `feeds::fetch`, the response helpers pattern from `api/event.rs` (`unprocessable`/`bad_request`/`not_found`).
- Produces handlers: `create_calendar`, `list_calendars_handler`, `get_calendar`, `patch_calendar`, `delete_calendar_handler`, `refresh_calendar`. Routes registered in `lib.rs`.

- [ ] **Step 1: Write the failing API + e2e tests**

Create `crates/amity-service/tests/calendar_api.rs`. Reuse the `build_test_app`/`post_json`/`get`/`body_json` harness from `tests/event_api.rs`. Cover: `POST /calendars` → 201 with the row + `last_status:"never"`; blank name → 422; bad scheme → 422; `GET /calendars` lists it; `PATCH` toggles `enabled`; `DELETE` → 200 then `GET {id}` → 404; malformed id → 400. Then the **e2e**: because the real `refresh` would hit the network, drive ingestion through the sync job with an injected fetch (call `run_once` directly as in Task 4), then assert the event surfaces via `GET /surfacing/today?date=2099-05-01`.

```rust
#[tokio::test]
async fn create_lists_and_deletes_a_calendar() {
    let app = build_test_app().await;
    let create = post_json(app.clone(), "/api/v1/calendars",
        json!({ "name": "holidays", "url": "https://example.test/h.ics", "category": "holiday" })).await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let id = body_json(create).await["id"].as_str().unwrap().to_owned();

    let list = get(app.clone(), "/api/v1/calendars").await;
    assert_eq!(body_json(list).await["calendars"].as_array().unwrap().len(), 1);

    let del = get(app.clone(), &format!("/api/v1/calendars/{id}")).await; // GET then DELETE below
    assert_eq!(del.status(), StatusCode::OK);
}

#[tokio::test]
async fn blank_name_is_422_and_bad_scheme_is_422() {
    let app = build_test_app().await;
    let blank = post_json(app.clone(), "/api/v1/calendars",
        json!({ "name": "  ", "url": "https://example.test/x.ics" })).await;
    assert_eq!(blank.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let scheme = post_json(app, "/api/v1/calendars",
        json!({ "name": "bad", "url": "file:///etc/passwd" })).await;
    assert_eq!(scheme.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

*(Add the e2e surfacing test using `run_once` with an injected fixture, asserting the ingested event appears on `/surfacing/today`, mirroring `event_api.rs::create_event_then_it_surfaces_on_today`.)*

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p amity-service --test calendar_api`
Expected: FAIL — handlers/routes undefined (404s / compile error).

- [ ] **Step 3: Implement the handlers**

Mirror `api/event.rs`. Request/response structs: `CreateCalendarRequest { name, url, category: Option<String> }`, `CalendarResponse { id, name, url, category, enabled, created_at, last_status, last_synced_at, last_error, event_count }`, list envelope `{ calendars: [...] }`. `create_calendar` builds via `CalendarBuilder` (map `CalendarError` → `unprocessable`), inserts, returns 201. `get_calendar`/`patch_calendar`/`delete_calendar_handler` parse the id (`bad_request` on parse failure, `not_found` when the row is absent). `patch_calendar` takes `{ enabled: bool }`. `refresh_calendar` loads the calendar, calls `sync_one(&pool, now, &stored, &|url| feeds::fetch(url))`, returns the refreshed row (or its sync error status). `now = OffsetDateTime::now_utc()` at the handler edge (I/O layer may read the clock).

- [ ] **Step 4: Wire the routes**

In `lib.rs`, in the router chain (after the events block):

```rust
        // Calendar endpoints — subscribed read-only external ICS feeds (Task 5).
        .route("/api/v1/calendars", post(api::calendar::create_calendar))
        .route("/api/v1/calendars", get(api::calendar::list_calendars_handler))
        .route("/api/v1/calendars/{id}", get(api::calendar::get_calendar))
        .route("/api/v1/calendars/{id}", patch(api::calendar::patch_calendar))
        .route("/api/v1/calendars/{id}", axum::routing::delete(api::calendar::delete_calendar_handler))
        .route("/api/v1/calendars/{id}/refresh", post(api::calendar::refresh_calendar))
```

Add `pub mod calendar;` to `api/mod.rs` and update its route-list doc comment.

- [ ] **Step 5: Run tests, verify pass + full gate**

Run: `cargo test -p amity-service --test calendar_api`
Expected: PASS. Then the full gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -W clippy::pedantic`, `find crates -name '*.rs' | xargs bash scripts/comment-density.sh`, `cargo test --workspace`.

- [ ] **Step 6: Commit**

```bash
git add crates/amity-service/src/api/calendar.rs crates/amity-service/src/api/mod.rs crates/amity-service/src/lib.rs crates/amity-service/tests/calendar_api.rs
git commit -s -m "feat(task-5): add calendars API and end-to-end surfacing test"
```

---

## Task 6: ADR-0004 + project-map sync (docs)

**Files:**
- Create: `docs/adrs/0004-external-calendar-ingestion.md`
- Modify: `project-map.js`

- [ ] **Step 1: Write ADR-0004**

Follow the format of `docs/adrs/0002-recurrence-engine.md` (Context / Decision / Consequences). Record: the aggregator posture and why read-only ICS (not OAuth) is the MVP boundary; that this is the first outbound network call; the egress guards (scheme allow-list, 20s timeout, 5 MiB cap, 5-redirect bound, no household data sent); that feed URLs (with secret tokens) are stored plaintext in the local DB, consistent with local-first, and why per-column encryption would be theatre; and the decision to expand external RRULEs with the full `rrule` crate rather than the native subset validator.

- [ ] **Step 2: Sync the project map**

Edit ONLY `project-map.js` (the renderer stays generic). Set `project.updated` to the completion date. Update the `event` node: the ICS-ingestion part moves `planned → done`; the Reschedule/Annotate seam stays. Update the `privacy` node: change "no outbound data flow" to the precise statement — loopback-only service plus user-initiated read-only outbound fetches of configured feeds; add a `done` part for the egress guards. Add a `calendars`/ingestion mention to the `api` and `storage` nodes (event endpoints already listed; add the calendars endpoints and migration 0004). Update the roadmap `next` marker off Task 5. Validate the file: `node -e '...'` evaluating `window.PROJECT_MAP` and checking every dep/status/layer resolves (as done in prior map syncs).

- [ ] **Step 3: Commit**

```bash
git add docs/adrs/0004-external-calendar-ingestion.md project-map.js
git commit -s -F <message-file>   # multi-line message via file to avoid backtick execution
```

---

## Task 7 (optional): Hub Calendars list

Only if a read-only feed list in the hub is wanted now; it can be run only through `vite build` (no WebKit). Add `SubscribedCalendar` types + `listCalendars()` to `apps/hub-tauri/src/api.ts`, a `Calendars.tsx` view rendering name/category/last-status, and a Tauri command mirroring `surfacing_today`. Verify with `npm run build`. Commit `feat(task-5): add read-only Calendars list to the hub`.

---

## Self-Review

**1. Spec coverage** — every spec section maps to a task:
- §4 Calendar entity → Task 1. §5 parse/expand → Task 2. §6 storage/upsert/prune → Task 3. §7 fetch + sync job → Task 4. §8 API → Task 5. §9 surfacing (no change) → verified by Task 5's e2e test. §10 ADR + map → Task 6. §11 errors → `CalendarError`/`IcsError`/`FetchError`/`SyncStatus` across Tasks 1–4. §12 testing → tests in every task. §13 slicing → Tasks 1–6 (+7 optional).

**2. Placeholder scan** — no "TBD/TODO/handle errors appropriately". Two deliberate implementer-latitude points are bounded by concrete acceptance tests: the `ical` crate's exact accessor names (Task 2, contract = the `ParsedEvent` tests) and mirroring existing files for boilerplate (`recurrence_materialiser.rs`, `event.rs`, `event_api.rs`) — legitimate in an existing codebase, and each names the exact file + lines to follow.

**3. Type consistency** — names are stable across tasks: `Calendar`, `CalendarSyncState`, `SyncStatus`, `CalendarCategory`, `CalendarId` (Task 1) are consumed unchanged in Tasks 3–5; `ParsedEvent`/`parse_feed`/`expand_external` (Task 2) are consumed unchanged in Task 4; `upsert_external_events`/`prune_events_missing_from_feed` (Task 3) and `run_once`/`sync_one`/`FetchError` (Task 4) are consumed unchanged in Tasks 4–5. `StoredCalendar { calendar, sync }` is the single read model used by storage, the job, and the API.

**Verified against the codebase while writing this plan:** `EventBuilder::new()` is argless with a separate `.title(...)` (used correctly in the Task 3 tests), and `.start_at`/`.end_at`/`.all_day`/`.source`/`.now`/`.build` all exist (`crates/amity-core/src/event.rs:252-353`); `amity_storage::event::list_events` is `pub` (`event.rs:221`); `axum::routing::delete` is the correct import (not yet used elsewhere in the service). **Still to confirm at implementation time:** the `ical` crate version's exact property accessors before Task 2 (contract remains the `ParsedEvent` tests).
