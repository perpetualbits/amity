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

// `IcalEvent`/`Property` are the parsed-component types we read fields off
// of; `IcalParser` is the entry point that turns bytes into `IcalCalendar`s.
use ical::IcalParser;
use ical::parser::ical::component::IcalEvent;
use ical::property::Property;
// `thiserror` gives `IcsError` a `std::error::Error` impl with one line.
use thiserror::Error;
// The project's canonical date/time type; see recurrence_materialiser.rs for
// why `time` (not `chrono`) is preferred everywhere outside the rrule boundary.
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

// `RecurrenceRule::default_timezone()` is the single source of truth for the
// "Europe/Amsterdam" default used across the codebase when no explicit
// timezone is available — reused here rather than duplicating the literal.
use crate::recurrence::RecurrenceRule;

/// One VEVENT extracted from an external ICS feed, reduced to the fields
/// Amity actually renders (brief §7 scope). Everything else in the source
/// VEVENT — VALARM, RECURRENCE-ID, organizer/attendee — is dropped.
// `PartialEq`/`Eq` support direct comparison in tests (see the
// `a_non_recurring_event_expands_to_its_single_start` assertion below);
// `Clone` lets callers keep an owned copy without re-parsing the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    /// The iCalendar UID — stable identity for this event across syncs.
    pub uid: String,
    /// The event title (SUMMARY).
    pub summary: String,
    /// The first (or only, for non-recurring events) instance start.
    pub start: OffsetDateTime,
    /// The instance end, if the source supplied DTEND.
    pub end: Option<OffsetDateTime>,
    /// True when DTSTART carried `VALUE=DATE` (a whole-day event, no time).
    pub all_day: bool,
    /// The raw RRULE value (no `RRULE:` prefix), if this VEVENT recurs.
    pub rrule: Option<String>,
    /// Instances to exclude from the recurrence, parsed from EXDATE lines.
    pub exdates: Vec<OffsetDateTime>,
    /// The TZID param on DTSTART, if the source specified one explicitly.
    pub tzid: Option<String>,
}

/// Errors from parsing an ICS feed. Per-event problems are not fatal (see
/// module docs) — this only covers the payload being unrecognisable at all.
// `thiserror::Error` derives `std::error::Error` from the `#[error(...)]`
// message below; `PartialEq`/`Eq` let tests assert on the variant directly
// via `matches!` without a manual `Debug`-string comparison.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IcsError {
    /// The payload has no `BEGIN:VCALENDAR` line — it is not iCalendar.
    #[error("payload does not contain BEGIN:VCALENDAR — not an iCalendar feed")]
    NotCalendar,
}

/// Parse raw ICS feed text into the events Amity cares about.
///
/// A feed with no `BEGIN:VCALENDAR` anywhere is rejected outright. Within a
/// recognised feed, each VEVENT is parsed independently; one malformed VEVENT
/// (missing UID/SUMMARY/DTSTART, or an unparseable DTSTART) is dropped and
/// logged, never fatal to the rest of the feed. An empty result is valid —
/// a feed with zero events is not an error.
///
/// # Errors
///
/// Returns [`IcsError::NotCalendar`] if the payload contains no
/// `BEGIN:VCALENDAR` line.
pub fn parse_feed(text: &str) -> Result<Vec<ParsedEvent>, IcsError> {
    // Cheap guard before handing the text to the parser: a real iCalendar
    // payload always opens with this line. This is what distinguishes "not
    // a calendar at all" (fatal) from "a calendar with a broken event"
    // (non-fatal, handled per-VEVENT below).
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(IcsError::NotCalendar);
    }

    // `&[u8]` implements `BufRead` directly, so no `BufReader` wrapper is
    // needed. The parser yields one `Result<IcalCalendar, _>` per VCALENDAR
    // block found in the text.
    let parser = IcalParser::new(text.as_bytes());

    // Accumulate the events across every VCALENDAR block in the payload
    // (feeds normally contain exactly one, but nothing stops there being
    // more, and the parser is happy to hand back several).
    let mut events = Vec::new();
    // Each loop iteration is one `Result<IcalCalendar, ParserError>`.
    for calendar in parser {
        // A calendar-level parse error (malformed structure) is skipped the
        // same way a bad VEVENT is — the guard above already established
        // this payload claims to be iCalendar, so we do our best with what
        // parses and drop the rest.
        let Ok(calendar) = calendar else {
            tracing::debug!("skipping unparseable VCALENDAR block");
            continue;
        };
        // `calendar.events` is the `Vec<IcalEvent>` for this VCALENDAR;
        // VALARM/VTODO/VTIMEZONE components on the same calendar are ignored.
        for vevent in &calendar.events {
            // `parse_one` returns `None` (already logged) for a malformed
            // VEVENT; only successfully-parsed events are kept.
            if let Some(parsed) = parse_one(vevent) {
                events.push(parsed);
            }
        }
    }

    // An empty Vec is a valid result — a feed with zero events is not an
    // error, only a feed that isn't iCalendar at all is.
    Ok(events)
}

/// Extract one `ParsedEvent` from a raw `IcalEvent`, or `None` if a required
/// field is missing/unparseable. Wraps `parse_one_inner` purely to centralise
/// the "malformed VEVENT, log and skip" behaviour in one place.
fn parse_one(event: &IcalEvent) -> Option<ParsedEvent> {
    // Delegate to the `?`-based extractor; this wrapper's only job is the
    // logging side-effect on failure.
    let parsed = parse_one_inner(event);
    if parsed.is_none() {
        // `tracing::debug!` — not `warn!`/`error!` — because a malformed
        // VEVENT from a third-party feed is routine, not an operational
        // problem; the household never sees a broken sync over this.
        tracing::debug!("skipping malformed VEVENT (missing/unparseable UID, SUMMARY, or DTSTART)");
    }
    // Pass the (possibly `None`) result straight through to the caller.
    parsed
}

/// The actual extraction logic, written with `?` for early-return on any
/// missing/unparseable required field. Kept separate from `parse_one` so the
/// logging wrapper does not have to duplicate this control flow.
fn parse_one_inner(event: &IcalEvent) -> Option<ParsedEvent> {
    // UID and SUMMARY are simple required strings — no special parsing.
    // `?` on the `Option<&Property>` bails out if the property is absent at
    // all; the second `?` bails out if it is present but has no value.
    let uid = find_property(&event.properties, "UID")?.value.clone()?;
    let summary = find_property(&event.properties, "SUMMARY")?.value.clone()?;

    // DTSTART is required and drives both `start` and `all_day`.
    let dtstart_prop = find_property(&event.properties, "DTSTART")?;
    let dtstart_value = dtstart_prop.value.as_deref()?;
    // Keep the params slice around; both the datetime parse below and the
    // TZID lookup right after it need to inspect the same param list.
    let dtstart_params = dtstart_prop.params.as_deref();
    let (start, all_day) = parse_ics_datetime(dtstart_value, dtstart_params)?;

    // The TZID param on DTSTART, if any — carried forward on `ParsedEvent`
    // so `expand_external` can rebuild the same DTSTART;TZID=… block later.
    let tzid = param_value(dtstart_params, "TZID").map(str::to_owned);

    // DTEND is optional; a missing or unparseable value just means no end.
    // `and_then` short-circuits to `None` for either "no DTEND property" or
    // "DTEND present but unparseable" — both collapse to the same outcome.
    let end = find_property(&event.properties, "DTEND").and_then(|prop| {
        let value = prop.value.as_deref()?;
        // Only the instant is needed here; DTEND's own all-day flag (if any)
        // is not tracked separately from DTSTART's.
        parse_ics_datetime(value, prop.params.as_deref()).map(|(instant, _)| instant)
    });

    // RRULE is the raw rule string (no prefix) — stored as-is, expanded
    // later by `expand_external` via the full rrule crate.
    let rrule = find_property(&event.properties, "RRULE").and_then(|prop| prop.value.clone());

    // EXDATE may appear on multiple lines and/or hold a comma-separated list
    // of instants on one line (RFC 5545 §3.8.5.1). Flatten both cases.
    let exdates = event
        .properties
        .iter()
        // Keep only EXDATE lines; there may be zero, one, or several.
        .filter(|prop| prop.name.eq_ignore_ascii_case("EXDATE"))
        // Pair each line's raw value with its own params (VALUE=DATE/TZID
        // can differ per EXDATE line in principle, so params travel with
        // their own value rather than being hoisted out).
        .filter_map(|prop| prop.value.as_deref().map(|v| (v, prop.params.as_deref())))
        // Each line's value may itself list several comma-separated dates;
        // parse every one and drop any that fail to parse individually.
        .flat_map(|(value, params)| {
            value
                .split(',')
                .filter_map(move |single| parse_ics_datetime(single, params))
                // Only the instant matters for exclusion, not its all-day flag.
                .map(|(instant, _)| instant)
        })
        .collect();

    // All required fields resolved — assemble the public result type.
    Some(ParsedEvent {
        uid,
        summary,
        start,
        end,
        all_day,
        rrule,
        exdates,
        tzid,
    })
}

/// Find a property by name, case-insensitively (real feeds emit uppercase
/// per RFC 5545, but defensive parsing tolerates lowercase too).
fn find_property<'a>(properties: &'a [Property], name: &str) -> Option<&'a Property> {
    // Linear scan: VEVENT property lists are small (a dozen or so lines),
    // so this is not worth a HashMap.
    properties
        .iter()
        .find(|prop| prop.name.eq_ignore_ascii_case(name))
}

/// Read the first value of a named param (e.g. `TZID`, `VALUE`) out of the
/// `(name, values)` tuple list the `ical` crate hands back for a property.
fn param_value<'a>(params: Option<&'a [(String, Vec<String>)]>, key: &str) -> Option<&'a str> {
    // `params?` short-circuits to `None` when the property had no params at
    // all (the common case for a bare `SUMMARY:...` line).
    params?
        .iter()
        // Each tuple is (param name, [param values]); RFC 5545 allows a
        // param to carry a list, but every param this module reads (TZID,
        // VALUE) only ever has one value in practice.
        .find(|(name, _)| name.eq_ignore_ascii_case(key))?
        .1
        .first()
        .map(String::as_str)
}

/// True when the named param is present and its (first) value matches, e.g.
/// detecting `VALUE=DATE` on a DTSTART/DTEND/EXDATE line.
fn has_param(params: Option<&[(String, Vec<String>)]>, key: &str, value: &str) -> bool {
    // Case-insensitive on the value too: `VALUE=date` is non-conformant but
    // harmless to accept.
    param_value(params, key).is_some_and(|found| found.eq_ignore_ascii_case(value))
}

/// Split an 8-digit `YYYYMMDD` string into its numeric components.
fn split_ymd(value: &str) -> Option<(i32, u8, u8)> {
    // Reject anything that isn't exactly 8 ASCII digits worth of length up
    // front; the slice-and-parse below would otherwise panic on a short
    // string via an out-of-bounds range.
    if value.len() != 8 {
        return None;
    }
    // `get` (not indexing) returns `None` instead of panicking on a
    // malformed byte boundary; `parse().ok()` turns a non-numeric slice
    // into `None` rather than propagating a `ParseIntError`.
    let year = value.get(0..4)?.parse().ok()?;
    let month = value.get(4..6)?.parse().ok()?;
    let day = value.get(6..8)?.parse().ok()?;
    Some((year, month, day))
}

/// Parse one ICS datetime value into `(instant, all_day)`.
///
/// Two forms are handled (brief §7 scope):
///   - `YYYYMMDD` with a `VALUE=DATE` param — an all-day date. Time is set
///     to local midnight in the TZID (or default) zone; `all_day = true`.
///   - `YYYYMMDDTHHMMSS[Z]` — a timed instant. A trailing `Z` means UTC;
///     otherwise the TZID param (or the project default) supplies the zone.
fn parse_ics_datetime(
    value: &str,
    params: Option<&[(String, Vec<String>)]>,
) -> Option<(OffsetDateTime, bool)> {
    // Branch on the all-day marker first: an all-day value has a completely
    // different shape (8 digits, no 'T', no time-of-day) from a timed one.
    if has_param(params, "VALUE", "DATE") {
        // All-day: date only, no time component in the source value.
        let (year, month, day) = split_ymd(value)?;
        // Fall back to the project-wide default zone when the source has no
        // explicit TZID (all-day values are usually "floating" per RFC 5545,
        // i.e. zone-less, but we still need *some* zone to anchor midnight).
        let tz = param_value(params, "TZID").unwrap_or(RecurrenceRule::default_timezone());
        let instant = local_instant(tz, year, month, day, 0, 0, 0)?;
        return Some((instant, true));
    }

    // Timed form: "YYYYMMDDTHHMMSS" optionally followed by "Z".
    let (date_part, time_part) = value.split_once('T')?;
    let (year, month, day) = split_ymd(date_part)?;
    // A trailing 'Z' (Zulu time) means the value is already UTC.
    let is_utc = time_part.ends_with('Z');
    // Strip the marker before parsing the fixed-width HHMMSS digits.
    let time_digits = time_part.trim_end_matches('Z');
    if time_digits.len() != 6 {
        return None;
    }
    let hour = time_digits.get(0..2)?.parse().ok()?;
    let minute = time_digits.get(2..4)?.parse().ok()?;
    let second = time_digits.get(4..6)?.parse().ok()?;

    if is_utc {
        // `Z` is unambiguous: build directly in UTC, no rrule/tz round-trip
        // needed.
        let date = Date::from_calendar_date(year, month_from_u8(month)?, day).ok()?;
        let time = Time::from_hms(hour, minute, second).ok()?;
        // `assume_utc` is exact here — the value really is UTC, not a guess.
        let instant = PrimitiveDateTime::new(date, time).assume_utc();
        return Some((instant, false));
    }

    // No `Z`: resolve against the explicit TZID param, or the project
    // default timezone (Europe/Amsterdam) when the feed omits one entirely.
    let tz = param_value(params, "TZID").unwrap_or(RecurrenceRule::default_timezone());
    let instant = local_instant(tz, year, month, day, hour, minute, second)?;
    Some((instant, false))
}

/// Convert a `1..=12` month number into `time::Month`.
///
/// A tiny named wrapper (rather than an inline `Month::try_from(...).ok()`)
/// so the call site in the UTC branch above reads as "the month", not as a
/// fallible conversion the reader has to double back on.
fn month_from_u8(month: u8) -> Option<Month> {
    Month::try_from(month).ok()
}

/// Resolve a local wall-clock date/time in a named IANA timezone to the
/// correct DST-aware `OffsetDateTime`, by round-tripping through the rrule
/// crate rather than re-implementing timezone-rule lookup.
///
/// The rrule crate requires at least one RRULE or RDATE line to parse a
/// `RRuleSet` (a bare DTSTART is rejected), so this builds a trivial
/// `FREQ=DAILY;COUNT=1` rule purely to satisfy that requirement — it always
/// produces exactly one instance, the DTSTART itself, expressed with the
/// zone's correct offset for that specific date (handling DST correctly).
/// This mirrors the DTSTART-line construction in
/// `recurrence_materialiser::materialise_instances`.
fn local_instant(
    tz: &str,
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> Option<OffsetDateTime> {
    // Zero-padded fixed-width fields exactly match the RFC 5545 basic
    // date-time format the rrule parser expects.
    let block = format!(
        "DTSTART;TZID={tz}:{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}\nRRULE:FREQ=DAILY;COUNT=1"
    );
    // A bad TZID name or malformed block surfaces as a parse error here;
    // callers treat that the same as any other unparseable datetime (None).
    let rrule_set: rrule::RRuleSet = block.parse().ok()?;
    // COUNT=1 guarantees at most one instance — no safety cap needed.
    let mut instances = rrule_set.all(1).dates.into_iter();
    // Convert the single chrono instant back to the project's time type.
    instances.next().map(chrono_dt_to_time_odt)
}

/// Expand a parsed event into concrete instants within `[from, to]`.
///
/// A non-recurring event expands to its own `start` when that falls inside
/// the window, or to nothing otherwise. A recurring event is expanded via
/// the full rrule crate (see module docs for why this bypasses the native
/// `recurrence.rs` validator), honouring `exdates` and applying the same
/// 1000-instance safety cap as `materialise_instances`.
#[must_use]
pub fn expand_external(
    event: &ParsedEvent,
    from: OffsetDateTime,
    to: OffsetDateTime,
) -> Vec<OffsetDateTime> {
    let Some(rrule_str) = event.rrule.as_deref() else {
        // No RRULE: the event is its own single instance. Include it only
        // if that single instant actually falls inside the query window.
        return if event.start >= from && event.start <= to {
            vec![event.start]
        } else {
            vec![]
        };
    };

    // Determine which zone name to pair with the DTSTART's local components.
    // `event.start` already carries the correct offset for its zone (resolved
    // during parsing), so a zero UTC offset unambiguously means the source
    // used `Z` — no IANA zone (Amsterdam, or any other) is ever at +00:00,
    // so this check cannot misclassify a real Amsterdam instant as UTC.
    let tz = if event.start.offset() == UtcOffset::UTC {
        "UTC"
    } else {
        event
            .tzid
            .as_deref()
            .unwrap_or(RecurrenceRule::default_timezone())
    };

    // Build the DTSTART;TZID=…:…\nRRULE:… block from the event's local-time
    // components, exactly as `materialise_instances` does — see
    // recurrence_materialiser.rs:74-140 for the pattern this mirrors.
    let block = format!(
        "DTSTART;TZID={tz}:{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}\nRRULE:{rrule_str}",
        y = event.start.year(),
        mo = event.start.month() as u8,
        d = event.start.day(),
        h = event.start.hour(),
        mi = event.start.minute(),
        s = event.start.second(),
    );

    // A malformed RRULE (or bad TZID) yields no instances rather than a
    // panic — external feeds are read-only and must not crash the sync job.
    let Ok(rrule_set) = block.parse::<rrule::RRuleSet>() else {
        return Vec::new();
    };

    // Compare unix timestamps rather than `OffsetDateTime`s directly to
    // avoid any ambiguity from mixed offsets between `from`/`to` and the
    // generated instances — a plain integer comparison is unambiguous.
    let from_unix = from.unix_timestamp();
    let to_unix = to.unix_timestamp();

    // EXDATE compares by calendar date (the common case: skip a single
    // occurrence's day), matching the holiday-skip comparison used in
    // `materialise_instances`.
    let exdate_days: std::collections::HashSet<Date> =
        event.exdates.iter().map(|instant| instant.date()).collect();

    // Same 1000-instance safety cap as materialise_instances — far beyond
    // any realistic feed's instance count within a bounded sync window.
    rrule_set
        .all(1000)
        .dates
        .into_iter()
        .map(chrono_dt_to_time_odt)
        .filter(|instant| {
            let ts = instant.unix_timestamp();
            ts >= from_unix && ts <= to_unix
        })
        .filter(|instant| !exdate_days.contains(&instant.date()))
        .collect()
}

/// Convert a `chrono::DateTime<rrule::Tz>` to `time::OffsetDateTime`.
///
/// Identical in approach to `recurrence_materialiser::chrono_dt_to_time_odt`
/// (duplicated rather than shared because that function is private to its
/// module) — see that module's doc comment for the step-by-step rationale.
fn chrono_dt_to_time_odt(dt: chrono::DateTime<rrule::Tz>) -> OffsetDateTime {
    // UTC unix timestamp (seconds + sub-second nanos).
    let utc_secs = dt.timestamp();
    let utc_nanos = dt.timestamp_subsec_nanos();

    // The offset is the gap between the zone's local wall-clock reading and
    // the UTC reading of the same instant.
    let local_naive = dt.naive_local();
    let utc_naive = dt.naive_utc();
    // Bounded by ±14h = ±50400s, always within i32 — no truncation risk.
    #[allow(clippy::cast_possible_truncation)]
    let offset_secs = (local_naive - utc_naive).num_seconds() as i32;
    let utc_offset = UtcOffset::from_whole_seconds(offset_secs)
        // A valid IANA timezone always yields a valid (±26h-bounded) offset.
        .expect("rrule timezone offset is always within the valid ±26:00:00 range");

    let total_nanos = i128::from(utc_secs) * 1_000_000_000 + i128::from(utc_nanos);
    OffsetDateTime::from_unix_timestamp_nanos(total_nanos)
        // rrule only generates Gregorian-calendar dates, always representable.
        .expect("rrule DateTime is always within time::OffsetDateTime's range")
        // Re-express in the local offset so `.hour()` reads wall-clock time.
        .to_offset(utc_offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    // A minimal single timed VEVENT.
    const SINGLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:abc-1\r\nSUMMARY:Dentist\r\nDTSTART:20990202T090000Z\r\nDTEND:20990202T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    // An all-day VEVENT (VALUE=DATE).
    const ALL_DAY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:kd-1\r\nSUMMARY:King's Day\r\nDTSTART;VALUE=DATE:20990427\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    // A recurring VEVENT with a weekly rule.
    //
    // NOTE ON THE DTSTART DATE: the brief's Step 3 fixture used
    // DTSTART:20990106 (2099-01-06) with the comment "Mondays in Jan 2099
    // from the 6th: 6, 13, 20, 27 → 4 instances". 2099-01-06 is actually a
    // *Tuesday* (verified independently via both `time::Date::weekday()` and
    // Python's `datetime`), not a Monday — confirmed against the real `rrule`
    // crate too: `FREQ=WEEKLY;BYDAY=MO` anchored on that Tuesday correctly
    // produces its first occurrence on the *following* Monday (Jan 12), per
    // standard RFC 5545 semantics (DTSTART need not itself satisfy BYDAY;
    // the first instance is the earliest BYDAY on/after DTSTART). That yields
    // only 3 instances (12, 19, 26) before the test's Feb-1 window edge, not
    // 4 — a date-math bug in the brief's fixture, not a parsing/expansion
    // bug. Fixed here by moving DTSTART to 2099-01-05, an actual Monday, so
    // the fixture matches its own stated intent (4 weekly Mondays in
    // January). See the Task 2 report for the verification trail.
    const RECURRING: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:gym-1\r\nSUMMARY:Gym\r\nDTSTART:20990105T180000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

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
        assert_eq!(
            events.len(),
            1,
            "the good event survives, the bad one is dropped"
        );
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
        // Mondays in Jan 2099 from the 5th: 5, 12, 19, 26 → 4 instances.
        // (See the NOTE ON THE DTSTART DATE above the RECURRING fixture.)
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
