// recurrence_materialiser.rs — RFC 5545 RRULE instance generation.
//
// This module turns a `RecurrenceRule` into concrete `OffsetDateTime` values
// that can be stored in the `task_instances` table. The caller supplies the
// first-instance anchor (`dtstart`), a forward horizon, and an optional set
// of dates to skip (the holiday calendar). The materialiser returns the list
// of valid instances.
//
// Implementation notes:
//   • We delegate RFC 5545 parsing and DST-aware scheduling to the `rrule`
//     crate (see ADR-0002 for the rationale). The rrule crate uses `chrono`
//     internally; we convert its `DateTime<Tz>` output back to the project's
//     preferred `time::OffsetDateTime` type at the end of this module.
//   • `chrono` appears as a direct dependency (not just transitive) because
//     the conversion functions below call chrono methods explicitly. The
//     `time` crate remains preferred for all other date/time handling in this
//     codebase; chrono is confined to this one conversion layer.
//   • DST: all instances anchor to local wall-clock time. A weekly 08:00 task
//     keeps its 08:00 local time across spring/autumn transitions; only the
//     UTC offset changes. The rrule crate guarantees this behaviour when a
//     TZID is set in the DTSTART line.
//   • Holiday-skip: the caller passes `skip_dates` (a `HashSet<Date>`). The
//     materialiser filters out any instance whose date appears in that set.
//     The holiday data source does not exist yet — the service layer passes an
//     empty set via `stub_holiday_calendar()` until a real source is wired in.
//
// See `docs/adrs/0002-recurrence-engine.md` for the full design rationale.

// std collections for the holiday-skip parameter.
use std::collections::HashSet;

// `time` types used for input/output — the project's canonical date/time type.
use time::{Date, OffsetDateTime};

// `RecurrenceError` is shared between this module and `recurrence.rs`.
// `RecurrenceRule` is the input type.
use crate::recurrence::{RecurrenceError, RecurrenceRule};

// ─── Public API ───────────────────────────────────────────────────────────────

/// Generate the recurrence instances that fall within `[dtstart, horizon]`.
///
/// The caller supplies `dtstart` as a `time::OffsetDateTime` expressed in the
/// rule's local timezone (i.e., `dtstart.hour()` is the local wall-clock hour,
/// and `dtstart.offset()` carries the correct UTC offset for that instant). The
/// materialiser builds the iCalendar `DTSTART;TZID=…` line directly from these
/// local-time components, so the rrule engine can anchor instances to the
/// correct wall-clock time.
///
/// If `dtstart` is stored in UTC (`+00:00` offset), the local time will be
/// interpreted as UTC which is probably wrong; use the correct local-time
/// `OffsetDateTime` when calling this function.
///
/// # Parameters
///
/// - `rule`: the `RecurrenceRule` (RRULE string + IANA timezone).
/// - `dtstart`: the first candidate instance. Instances on or after this
///   datetime are included; instances before it are not.
/// - `horizon`: the forward limit. Instances with a UTC timestamp later than
///   `horizon` are excluded.
/// - `skip_dates`: dates to omit from the result. Pass an empty set if no
///   dates should be skipped. Typically the household's holiday calendar for
///   tasks tagged `chore` or `household`.
///
/// # Errors
///
/// Returns [`RecurrenceError::UnsupportedFeature`] or
/// [`RecurrenceError::UnsupportedFrequency`] if the rule uses out-of-scope
/// RRULE features.
///
/// Returns [`RecurrenceError::InvalidRule`] if the RRULE string or timezone
/// name is syntactically invalid.
#[allow(clippy::implicit_hasher)]
pub fn materialise_instances(
    rule: &RecurrenceRule,
    dtstart: OffsetDateTime,
    horizon: OffsetDateTime,
    skip_dates: &HashSet<Date>,
) -> Result<Vec<OffsetDateTime>, RecurrenceError> {
    // Validate the rule before attempting to parse it with the rrule crate.
    // This surfaces unsupported features with Amity-specific error messages
    // rather than the generic rrule parse errors.
    rule.validate()?;

    // Build the iCalendar DTSTART+RRULE block. The DTSTART line uses the
    // local-time components from `dtstart` (hour, minute, second in the
    // stored UTC offset) so that the rrule engine anchors instances correctly.
    //
    // Format: "DTSTART;TZID=Europe/Amsterdam:20260601T080000\nRRULE:FREQ=WEEKLY;BYDAY=TH"
    // The rrule crate parses this combined string as a standard iCalendar block.
    let full_rule_str = format!(
        "DTSTART;TZID={tz}:{y:04}{mo:02}{d:02}T{h:02}{m:02}{s:02}\nRRULE:{rule_str}",
        tz = rule.timezone,
        // year/month/day/hour/minute/second are the LOCAL time values in the
        // offset stored on `dtstart` — exactly what the DTSTART line needs.
        y = dtstart.year(),
        mo = dtstart.month() as u8,
        d = dtstart.day(),
        h = dtstart.hour(),
        m = dtstart.minute(),
        s = dtstart.second(),
        rule_str = rule.rrule,
    );

    // Parse the block through the rrule crate. This validates the RRULE
    // syntax and the timezone name. A bad timezone name surfaces here.
    let rrule_set: rrule::RRuleSet = full_rule_str.parse().map_err(|e| {
        // Convert the rrule error to a string so callers are not coupled to
        // the rrule crate's error type — if rrule changes its error format,
        // only this conversion needs updating.
        RecurrenceError::InvalidRule(format!("{e}"))
    })?;

    // The horizon as a UTC unix timestamp for efficient filtering in the loop.
    // We compare timestamps (not OffsetDateTimes) to avoid timezone ambiguity
    // in the comparison; unix timestamps are always in UTC.
    let horizon_unix = horizon.unix_timestamp();

    // Materialise all instances up to a safety cap. With a 60-day horizon
    // and only daily-or-rarer frequencies supported, 1000 is far beyond any
    // realistic instance count. The cap prevents infinite loops for
    // hypothetical rules that produce many instances.
    let result = rrule_set.all(1000);

    // Convert each rrule DateTime to time::OffsetDateTime, filter by horizon
    // and skip_dates, and collect into the result vector.
    let instances = result
        .dates
        .into_iter()
        // Keep only instances within the materialisation window.
        .filter(|dt| dt.timestamp() <= horizon_unix)
        // Convert from the rrule/chrono type to the project's time type.
        // This is the boundary where chrono types are handled; all other code
        // in this crate uses time::OffsetDateTime exclusively.
        .map(chrono_dt_to_time_odt)
        // Apply the holiday (or other) skip set. Dates in this set are omitted
        // regardless of the RRULE. See ADR-0002 §holiday-skip for rationale.
        .filter(|dt| !skip_dates.contains(&dt.date()))
        .collect();

    Ok(instances)
}

/// Return a stub holiday calendar — an empty set of dates.
///
/// This function is the placeholder for future NL public holiday data.
/// The materialiser calls this when it needs a skip set for tasks tagged
/// `chore` or `household`.
///
/// TODO: load real NL public holidays from a bundled ICS or hardcoded list.
/// See the "holiday calendar source" item in the task-2 scope guardrails.
#[must_use]
pub fn stub_holiday_calendar() -> HashSet<Date> {
    // Empty set: no dates are skipped until the real data source is wired in.
    HashSet::new()
}

// ─── Conversion helpers ───────────────────────────────────────────────────────

/// Convert a `chrono::DateTime<rrule::Tz>` to `time::OffsetDateTime`.
///
/// The `dates` field of `rrule::RRuleResult` contains `chrono::DateTime<rrule::Tz>`
/// values (the type alias `rrule::DateTime` is `pub(crate)` and not accessible
/// outside the rrule crate). This function converts each such value back to
/// the project's canonical `time::OffsetDateTime` type.
///
/// It is the sole place in `amity-core` where chrono types are handled
/// directly; everywhere else the project uses `time::OffsetDateTime`.
///
/// The conversion:
///   1. Reads the UTC unix timestamp (seconds + nanoseconds).
///   2. Computes the UTC offset by comparing the local and UTC naive datetimes.
///   3. Builds a UTC `OffsetDateTime` and re-expresses it in the local offset.
///
/// The result correctly represents the wall-clock time: an instance at
/// 08:00 Amsterdam time will have `.hour() == 8` and an offset of either
/// `+01:00` (CET, winter) or `+02:00` (CEST, summer).
fn chrono_dt_to_time_odt(dt: chrono::DateTime<rrule::Tz>) -> OffsetDateTime {
    // Step 1: extract the UTC unix timestamp.
    // `.timestamp()` and `.timestamp_subsec_nanos()` are standard chrono
    // methods available on `DateTime<Tz>` without additional imports.
    let utc_secs = dt.timestamp();
    let utc_nanos = dt.timestamp_subsec_nanos();

    // Step 2: compute the UTC offset in seconds.
    // `naive_local()` returns the wall-clock time in the timezone;
    // `naive_utc()` returns the UTC time. Their difference is the offset.
    // Both return `chrono::NaiveDateTime`; subtraction yields `chrono::Duration`.
    let local_naive = dt.naive_local();
    let utc_naive = dt.naive_utc();
    // The offset is always bounded by ±14 hours = ±50400 seconds, well within i32.
    // The cast cannot truncate because no IANA timezone has a larger offset.
    #[allow(clippy::cast_possible_truncation)]
    let offset_secs = (local_naive - utc_naive).num_seconds() as i32;

    // Step 3: build the time::OffsetDateTime in UTC then re-apply the local offset.
    let utc_offset = time::UtcOffset::from_whole_seconds(offset_secs)
        // A valid IANA timezone always produces a valid UTC offset; this cannot fail
        // in practice. The expect message is a diagnostic hint if it ever does.
        .expect("rrule timezone offset is always within the valid ±26:00:00 range");

    // Total nanoseconds since the Unix epoch for sub-second precision.
    let total_nanos = i128::from(utc_secs) * 1_000_000_000 + i128::from(utc_nanos);
    time::OffsetDateTime::from_unix_timestamp_nanos(total_nanos)
        // The rrule crate only generates dates within the Gregorian calendar;
        // they are always within time::OffsetDateTime's representable range.
        .expect("rrule DateTime is always within time::OffsetDateTime's range")
        // Convert from UTC to the local offset so that .hour() returns the
        // wall-clock time, not the UTC time.
        .to_offset(utc_offset)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    /// Weekly-Thursday rule in Amsterdam timezone — the canonical test fixture.
    fn weekly_thu_amsterdam() -> RecurrenceRule {
        // FREQ=WEEKLY;BYDAY=TH generates instances every Thursday at the
        // wall-clock time supplied via dtstart.
        RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH", "Europe/Amsterdam")
    }

    /// A dtstart that is 08:00 Amsterdam summer time (CEST, +02:00).
    /// June 4 2026 is a Thursday in Amsterdam summer time.
    fn dtstart_summer() -> OffsetDateTime {
        // +02:00 is CEST (Central European Summer Time). The rrule engine
        // uses the TZID to anchor subsequent instances at 08:00 local.
        datetime!(2026-06-04 08:00:00 +02:00)
    }

    /// A dtstart in Amsterdam winter time (CET, +01:00).
    /// October 1 2026 is a Thursday.
    fn dtstart_winter() -> OffsetDateTime {
        // +02:00 because Oct 1 is still in CEST (DST ends Oct 25 2026).
        datetime!(2026-10-01 08:00:00 +02:00)
    }

    #[test]
    fn materialises_weekly_instances_in_correct_order() {
        // Basic smoke test: weekly Thursday instances must be in ascending order
        // and separated by exactly one week (7 * 24h = 604800 seconds).
        let rule = weekly_thu_amsterdam();
        // dtstart is a Thursday 08:00 CEST; horizon is 4 weeks later.
        let dtstart = dtstart_summer();
        let horizon = dtstart + time::Duration::weeks(4);

        // No holidays to skip in this test.
        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("valid rule must materialise without error");

        // 4 weeks forward from a Thursday gives 5 Thursdays: 0, 1, 2, 3, 4 weeks out.
        assert_eq!(instances.len(), 5, "expected 5 weekly Thursday instances");

        // Verify consecutive instances are exactly one week apart.
        for window in instances.windows(2) {
            let gap_secs = window[1].unix_timestamp() - window[0].unix_timestamp();
            // One week = 604800 seconds. DST transitions do not affect wall-clock
            // spacing for weekly rules (the time difference is measured in UTC).
            assert_eq!(
                gap_secs, 604_800,
                "instances must be exactly one week apart"
            );
        }
    }

    #[test]
    fn dst_autumn_fallback_preserves_wall_clock_time() {
        // DST KEY TEST: the autumn fallback in Amsterdam is 2026-10-25.
        // Before: CEST +02:00. After: CET +01:00.
        // A weekly Thursday task must stay at 08:00 local time; only the UTC
        // offset should change.
        //
        // Thursdays in October 2026: 1, 8, 15, 22, 29.
        // DST fallback: 2026-10-25 (Sunday).
        // Oct 22 = before fallback (+02:00 CEST), Oct 29 = after fallback (+01:00 CET).
        let rule = weekly_thu_amsterdam();
        let dtstart = dtstart_winter(); // 2026-10-01 08:00 +02:00
        // Horizon set to cover all of October.
        let horizon = datetime!(2026-11-01 00:00:00 +01:00);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("valid rule must materialise");

        // October has exactly 5 Thursdays in 2026.
        assert_eq!(instances.len(), 5, "expected 5 Thursdays in October 2026");

        // Oct 22 (index 3) — before the DST fallback, still CEST +02:00.
        let oct_22 = instances[3];
        // Wall-clock hour must be 08:00 in the local timezone.
        assert_eq!(oct_22.hour(), 8, "Oct 22 must be at 08:00 local time");
        // UTC offset must be +02:00 (CEST) before the fallback.
        assert_eq!(
            oct_22.offset(),
            time::UtcOffset::from_hms(2, 0, 0).unwrap(),
            "Oct 22 must be in CEST (+02:00)"
        );

        // Oct 29 (index 4) — after the DST fallback, CET +01:00.
        let oct_29 = instances[4];
        // Wall-clock hour must still be 08:00 — the rule is anchored to local time.
        assert_eq!(
            oct_29.hour(),
            8,
            "Oct 29 must be at 08:00 local time after DST fallback"
        );
        // UTC offset must be +01:00 (CET) after the fallback.
        assert_eq!(
            oct_29.offset(),
            time::UtcOffset::from_hms(1, 0, 0).unwrap(),
            "Oct 29 must be in CET (+01:00)"
        );

        // Verify the UTC gap is one week PLUS one hour (608400s) because the
        // autumn fallback adds one hour to the UTC difference: Oct 22 at
        // 08:00 CEST = 06:00 UTC; Oct 29 at 08:00 CET = 07:00 UTC.
        // If the recurrence engine were NOT timezone-aware, both instances would
        // be at the same UTC offset and the gap would be 604800s (plain 7 days).
        // 608400s confirms that DST anchoring is working correctly.
        let gap_secs = oct_29.unix_timestamp() - oct_22.unix_timestamp();
        assert_eq!(
            gap_secs, 608_400,
            "autumn fallback gap must be 7 days + 1h = 608400s"
        );
    }

    #[test]
    fn dst_spring_forward_preserves_wall_clock_time() {
        // DST KEY TEST: the spring forward in Amsterdam is 2027-03-28.
        // Before: CET +01:00. After: CEST +02:00.
        // Thursdays: Mar 25 (before, +01:00), Apr 1 (after, +02:00).
        let rule = weekly_thu_amsterdam();
        // Mar 18 2027 is a Thursday at 08:00 CET (+01:00).
        let dtstart = datetime!(2027-03-18 08:00:00 +01:00);
        // Horizon covers first week of April.
        let horizon = datetime!(2027-04-08 00:00:00 +02:00);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("valid rule must materialise");

        // Should include Mar 18, Mar 25, Apr 1 (3 Thursdays).
        assert_eq!(
            instances.len(),
            3,
            "expected 3 Thursdays across the spring-forward"
        );

        // Mar 25 (index 1) — before spring-forward, CET +01:00.
        let mar_25 = instances[1];
        assert_eq!(mar_25.hour(), 8, "Mar 25 must be at 08:00 local time");
        assert_eq!(
            mar_25.offset(),
            time::UtcOffset::from_hms(1, 0, 0).unwrap(),
            "Mar 25 must be in CET (+01:00)"
        );

        // Apr 1 (index 2) — after spring-forward, CEST +02:00.
        let apr_1 = instances[2];
        assert_eq!(
            apr_1.hour(),
            8,
            "Apr 1 must be at 08:00 local time after spring-forward"
        );
        assert_eq!(
            apr_1.offset(),
            time::UtcOffset::from_hms(2, 0, 0).unwrap(),
            "Apr 1 must be in CEST (+02:00)"
        );
    }

    #[test]
    fn holiday_skip_removes_matching_date() {
        // Tasks tagged `chore` or `household` skip NL public holidays. This
        // test verifies that a date in `skip_dates` is excluded from the result.
        let rule = weekly_thu_amsterdam();
        let dtstart = dtstart_summer(); // 2026-06-04 (Thu) 08:00 +02:00
        let horizon = dtstart + time::Duration::weeks(2);

        // Skip June 4 (the first Thursday — simulates a holiday on dtstart itself).
        let mut skip = HashSet::new();
        skip.insert(date!(2026 - 06 - 04));

        let instances = materialise_instances(&rule, dtstart, horizon, &skip).expect("valid rule");

        // Without the skip, there would be 3 instances (Jun 4, 11, 18).
        // With Jun 4 skipped, there should be only 2.
        assert_eq!(
            instances.len(),
            2,
            "skipped date must not appear in results"
        );

        // Verify the first instance is Jun 11, not Jun 4.
        assert_eq!(instances[0].date(), date!(2026 - 06 - 11));
    }

    #[test]
    fn count_terminates_instances() {
        // COUNT=3 must stop the materialiser after exactly 3 instances,
        // even if the horizon extends further.
        //
        // This verifies that the safety cap of 1000 in `rrule_set.all(1000)`
        // is never reached by COUNT-bounded rules — the rrule crate terminates
        // internally at COUNT without materialising all 1000 slots.
        let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH;COUNT=3", "Europe/Amsterdam");
        let dtstart = dtstart_summer(); // 2026-06-04
        // Horizon is far in the future — COUNT should terminate first.
        let horizon = dtstart + time::Duration::weeks(20);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("COUNT rule must materialise");

        // Exactly 3 instances must be returned regardless of the long horizon.
        assert_eq!(
            instances.len(),
            3,
            "COUNT=3 must produce exactly 3 instances"
        );
    }

    #[test]
    fn until_terminates_instances() {
        // UNTIL terminates the recurrence at a specific datetime. Instances
        // after UNTIL must not appear even if the horizon extends further.
        //
        // The Jun 18 Thursday instance is at 08:00 Amsterdam CEST = 06:00 UTC.
        // UNTIL=20260619T000000Z (midnight Jun 19 UTC) is after 06:00 UTC Jun 18,
        // so the Jun 18 instance IS included. Jun 25 (at 06:00 UTC) is after UNTIL.
        let rule = RecurrenceRule::new(
            "FREQ=WEEKLY;BYDAY=TH;UNTIL=20260619T000000Z",
            "Europe/Amsterdam",
        );
        let dtstart = dtstart_summer(); // 2026-06-04 08:00 +02:00
        let horizon = dtstart + time::Duration::weeks(10);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("UNTIL rule must materialise");

        // Jun 4, Jun 11, Jun 18 are all before UNTIL (midnight Jun 19 UTC).
        // Jun 25 is after UNTIL and must not appear.
        assert_eq!(
            instances.len(),
            3,
            "UNTIL must terminate the series at Jun 18"
        );
    }

    #[test]
    fn positional_byday_last_friday_of_month() {
        // -1FR = "last Friday of the month" — a valid positional BYDAY.
        // Verify materialisation produces the correct dates.
        //
        // This exercises the rrule crate's handling of negative positional selectors,
        // which count from the end of the month. The Amity validation layer allows
        // negative positions like -1 and -2 on any weekday code.
        let rule = RecurrenceRule::new("FREQ=MONTHLY;BYDAY=-1FR", "Europe/Amsterdam");
        // First Friday of June 2026 is Jun 5. Last Friday is Jun 26.
        // dtstart on Jun 26 (last Friday of June, CET +02:00).
        let dtstart = datetime!(2026-06-26 08:00:00 +02:00);
        // Horizon: end of September 2026 to get Jun, Jul, Aug, Sep instances.
        let horizon = datetime!(2026-09-30 23:59:59 +02:00);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("positional BYDAY rule must materialise");

        // Last Fridays: Jun 26, Jul 31, Aug 28, Sep 25 → 4 instances.
        assert_eq!(instances.len(), 4, "expected last Friday of each month");

        // Each instance must be on a Friday.
        for inst in &instances {
            // time::OffsetDateTime::weekday() returns the day of the week.
            assert_eq!(
                inst.weekday(),
                time::Weekday::Friday,
                "every instance must be a Friday"
            );
        }
    }

    #[test]
    fn empty_result_when_horizon_before_dtstart() {
        // If the horizon is before or equal to dtstart, no instances can be
        // generated. The function must return an empty Vec, not an error.
        let rule = weekly_thu_amsterdam();
        let dtstart = dtstart_summer(); // 2026-06-04
        // Horizon set to BEFORE dtstart.
        let horizon = dtstart - time::Duration::days(1);

        let instances = materialise_instances(&rule, dtstart, horizon, &HashSet::new())
            .expect("empty result must not be an error");

        // No instances can fall within an empty window.
        assert!(
            instances.is_empty(),
            "horizon before dtstart must yield empty result"
        );
    }

    #[test]
    fn unsupported_feature_returns_error_before_parsing() {
        // Validation must happen before the rrule crate parses the string.
        // This ensures the error message is Amity-specific, not rrule's message.
        let rule = RecurrenceRule::new("FREQ=YEARLY;BYWEEKNO=10", "Europe/Amsterdam");
        let dtstart = dtstart_summer();
        let horizon = dtstart + time::Duration::weeks(10);

        let result = materialise_instances(&rule, dtstart, horizon, &HashSet::new());
        // Must fail with the unsupported-feature error, not a parse error.
        assert!(
            matches!(result, Err(RecurrenceError::UnsupportedFeature(_))),
            "expected UnsupportedFeature, got {result:?}"
        );
    }

    #[test]
    fn stub_holiday_calendar_is_empty() {
        // The stub returns an empty set; callers that pass it skip no dates.
        //
        // When a real NL public holiday source is wired in (ADR-0002 §holiday-skip),
        // this test must be updated or removed; the stub function itself becomes
        // a call to the real calendar loader.
        let cal = stub_holiday_calendar();
        assert!(cal.is_empty(), "stub calendar must be empty");
    }
}
