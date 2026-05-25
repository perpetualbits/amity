// recurrence.rs — RecurrenceRule type and validation.
//
// A RecurrenceRule stores the pattern for a recurring Task: the RRULE string
// (RFC 5545 format) and the IANA timezone that anchors the wall-clock time.
//
// This module owns:
//   • The `RecurrenceRule` domain type used on `Task.recurrence`.
//   • Validation logic that rejects RRULE features outside Amity's supported
//     subset. The subset is deliberately narrow — enough to cover household
//     chores and seasonal maintenance without exposing RRULE complexity.
//   • The `RecurrenceError` error type, shared with `recurrence_materialiser`.
//
// What is NOT in this module:
//   • Instance materialisation (that lives in `recurrence_materialiser.rs`).
//   • Storage format (the storage layer stores rrule + timezone as two TEXT
//     columns and reassembles this type on read).
//
// Supported RRULE features (brief §6.6 and task-2 spec §2):
//   FREQ  — DAILY, WEEKLY, MONTHLY, YEARLY (HOURLY/MINUTELY/SECONDLY rejected)
//   INTERVAL — any positive integer
//   BYDAY — weekday specs including positional prefixes (1MO, -1FR, etc.)
//   BYMONTHDAY — day-of-month constraints
//   UNTIL — hard cutoff date/time
//   COUNT — instance count limit
//
// Explicitly rejected (with a clear error):
//   BYWEEKNO, BYYEARDAY, BYHOUR, BYMINUTE, BYSECOND, BYSETPOS, WKST overrides.
//
// See docs/adrs/0002-recurrence-engine.md for the rationale behind the subset.

// Serde derives allow RecurrenceRule to round-trip through the JSON API and
// to be reconstructed from the database columns.
use serde::{Deserialize, Serialize};

// ─── RecurrenceRule ───────────────────────────────────────────────────────────

/// A recurrence pattern for a Task.
///
/// Stores the RRULE string (RFC 5545, §3.8.5) and the IANA timezone that
/// anchors the wall-clock time across DST transitions. Together they define
/// WHEN the task repeats — the materialiser turns this into concrete
/// `OffsetDateTime` instances.
///
/// The RRULE string must NOT include the `RRULE:` prefix. For example, use
/// `"FREQ=WEEKLY;BYDAY=TH"` not `"RRULE:FREQ=WEEKLY;BYDAY=TH"`. The prefix
/// is added by the materialiser when it builds the full iCalendar property.
///
/// The `timezone` string must be a valid IANA timezone name understood by the
/// `rrule` crate (e.g. `"Europe/Amsterdam"`, `"UTC"`, `"America/New_York"`).
/// Amity defaults to `"Europe/Amsterdam"` per brief §7 ("native hub events
/// default to Europe/Amsterdam").
///
/// # Examples
///
/// ```rust
/// # use amity_core::recurrence::RecurrenceRule;
/// // Every Thursday — the classic bin-night chore pattern.
/// let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH", "Europe/Amsterdam");
/// assert!(rule.validate().is_ok());
/// ```
///
/// See `docs/amity_brief.md` §6.6 for the recurrence policy.
// Debug for test inspection; Clone so the rule can be stored on Task and
// also passed to the materialiser without move issues; PartialEq for test
// assertions; Serialize/Deserialize for API and database round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurrenceRule {
    /// RFC 5545 RRULE string, without the `RRULE:` prefix.
    ///
    /// Examples: `"FREQ=WEEKLY;BYDAY=TH"`, `"FREQ=MONTHLY;BYDAY=-1FR"`.
    /// This string is combined with the DTSTART line when the materialiser
    /// builds the full iCalendar property block.
    pub rrule: String,

    /// IANA timezone name for DST-correct instance generation.
    ///
    /// Wall-clock time is anchored to this timezone, so a task scheduled for
    /// 08:00 Amsterdam time stays at 08:00 across spring/autumn transitions
    /// (the UTC offset changes, the local time does not). See brief §6.6.
    pub timezone: String,
}

impl RecurrenceRule {
    /// Construct a new recurrence rule from an RRULE string and a timezone.
    ///
    /// This constructor does not validate the rule. Call [`validate`](Self::validate)
    /// before using the rule in a materialiser context.
    ///
    /// `rrule_str` must NOT include the `RRULE:` prefix.
    #[must_use]
    pub fn new(rrule_str: impl Into<String>, timezone: impl Into<String>) -> Self {
        // Accept both &str and String so callers don't need to call .to_owned().
        Self {
            rrule: rrule_str.into(),
            timezone: timezone.into(),
        }
    }

    /// Validate the RRULE string against the supported subset.
    ///
    /// Checks for features that Amity explicitly does not support and returns
    /// a descriptive error if any are found. Also validates that the frequency
    /// is one of DAILY, WEEKLY, MONTHLY, or YEARLY.
    ///
    /// # Errors
    ///
    /// Returns [`RecurrenceError::UnsupportedFeature`] if the RRULE uses
    /// `BYWEEKNO`, `BYYEARDAY`, `BYHOUR`, `BYMINUTE`, `BYSECOND`, `BYSETPOS`,
    /// or a non-default `WKST` value.
    ///
    /// Returns [`RecurrenceError::UnsupportedFrequency`] if `FREQ` is set to
    /// `HOURLY`, `MINUTELY`, or `SECONDLY` (sub-daily frequencies are not
    /// meaningful for household tasks and add DST complexity without benefit).
    pub fn validate(&self) -> Result<(), RecurrenceError> {
        // Normalise to uppercase so the checks are case-insensitive.
        // RFC 5545 requires property names to be in uppercase, but defensive
        // parsing tolerates lowercase from user input.
        validate_rrule_string(&self.rrule)
    }

    /// Return the default timezone used when no explicit timezone is provided.
    ///
    /// `"Europe/Amsterdam"` is the Amity default per brief §7.
    #[must_use]
    pub fn default_timezone() -> &'static str {
        // Amsterdam is the canonical home timezone for Amity v1.
        // Multi-household and multi-timezone support is post-MVP.
        "Europe/Amsterdam"
    }
}

impl Default for RecurrenceRule {
    /// Returns a weekly-on-Thursday rule in the default Amsterdam timezone.
    ///
    /// This default is primarily useful in tests and builder defaults;
    /// production callers always supply an explicit rule.
    fn default() -> Self {
        // Thursday is the most common chore day in the NL seasonal schedule.
        Self::new("FREQ=WEEKLY;BYDAY=TH", Self::default_timezone())
    }
}

// ─── Validation helpers ───────────────────────────────────────────────────────

/// Validate an RRULE string against the supported feature subset.
///
/// This function is the single source of truth for what Amity accepts. It is
/// called from [`RecurrenceRule::validate`] and should also be called in the
/// service layer when a client POSTs a new task with a recurrence field.
///
/// # Errors
///
/// See [`RecurrenceRule::validate`] for the list of error cases.
pub fn validate_rrule_string(rrule: &str) -> Result<(), RecurrenceError> {
    // Normalise to uppercase for case-insensitive matching. RFC 5545 requires
    // uppercase but we tolerate lowercase user input gracefully.
    let upper = rrule.to_uppercase();

    // Check for each explicitly unsupported feature. The list mirrors the
    // "explicitly out of scope" list in the task-2 spec §2 and ADR-0002.
    let unsupported = [
        "BYWEEKNO=",  // week-of-year; not needed for household tasks
        "BYYEARDAY=", // day-of-year; not needed for household tasks
        "BYHOUR=",    // sub-daily; tasks are not scheduled by the hour
        "BYMINUTE=",  // sub-daily; tasks are not scheduled by the minute
        "BYSECOND=",  // sub-daily; tasks are not scheduled by the second
        "BYSETPOS=",  // positional filtering; too complex for the UI
    ];

    for feature in &unsupported {
        // Using `contains` on the uppercase string is safe here because the
        // feature keywords are unambiguous (no keyword is a substring of another).
        if upper.contains(feature) {
            // Trim the trailing `=` for a cleaner error message.
            let name = feature.trim_end_matches('=');
            return Err(RecurrenceError::UnsupportedFeature(name.to_owned()));
        }
    }

    // WKST is allowed only with its RFC 5545 default value (MO = Monday).
    // A non-MO WKST shifts week boundaries in non-obvious ways that our UI
    // does not expose, and is not needed for any household use-case.
    if let Some(pos) = upper.find("WKST=") {
        // Extract the value after "WKST=" up to the next semicolon or end.
        let value = upper[pos + 5..].split(';').next().unwrap_or("");
        if value != "MO" {
            return Err(RecurrenceError::UnsupportedFeature(format!(
                "WKST={value} (only WKST=MO, the default, is supported)"
            )));
        }
    }

    // Validate that FREQ is in the supported set.
    // HOURLY/MINUTELY/SECONDLY are sub-daily and not meaningful for chores;
    // they would also complicate DST handling without any practical benefit.
    if let Some(pos) = upper.find("FREQ=") {
        // Extract the frequency value up to the next semicolon or end.
        let freq_value = upper[pos + 5..].split(';').next().unwrap_or("");
        match freq_value {
            // These four frequencies cover every household use-case.
            "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY" => {}
            // Sub-daily frequencies are rejected by design, not by oversight.
            other => {
                return Err(RecurrenceError::UnsupportedFrequency(other.to_owned()));
            }
        }
    }

    Ok(())
}

// ─── RecurrenceError ─────────────────────────────────────────────────────────

/// Errors that can occur when working with recurrence rules.
#[derive(Debug, thiserror::Error)]
pub enum RecurrenceError {
    /// The RRULE uses a feature outside Amity's supported subset.
    ///
    /// The string names the unsupported feature so the API can return a 422
    /// response that tells the client exactly what to fix.
    #[error("unsupported RRULE feature: {0}")]
    UnsupportedFeature(String),

    /// The RRULE uses a frequency not supported by Amity.
    ///
    /// Only DAILY, WEEKLY, MONTHLY, and YEARLY are accepted.
    /// The string is the value that was rejected.
    #[error("unsupported RRULE frequency: {0} (only DAILY/WEEKLY/MONTHLY/YEARLY are supported)")]
    UnsupportedFrequency(String),

    /// The RRULE string is syntactically invalid.
    ///
    /// This error wraps the underlying parse error from the `rrule` crate as a
    /// string so that callers are not coupled to the `rrule` crate's error type.
    #[error("invalid RRULE syntax: {0}")]
    InvalidRule(String),

    /// The timezone name is not a recognised IANA timezone.
    ///
    /// This can happen when a client sends a timezone string that is not in
    /// the IANA timezone database bundled with the `rrule` crate.
    #[error("unknown timezone: {0}")]
    UnknownTimezone(String),
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a weekly-Thursday rule in Amsterdam timezone.
    /// Used across multiple tests as the canonical "chore rule" example.
    fn weekly_thursday() -> RecurrenceRule {
        // FREQ=WEEKLY;BYDAY=TH — every Thursday, the Amity default example.
        RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH", "Europe/Amsterdam")
    }

    #[test]
    fn valid_weekly_rule_passes_validation() {
        // The most common household use-case: a weekly recurring chore.
        let rule = weekly_thursday();
        // Validation must succeed without errors.
        assert!(
            rule.validate().is_ok(),
            "weekly Thursday rule must be valid"
        );
    }

    #[test]
    fn valid_monthly_rule_passes_validation() {
        // "First Monday of each month" — a common chore pattern.
        // Positional BYDAY (1MO) is in the supported subset.
        // The `1` prefix selects the first occurrence of MO in the month.
        let rule = RecurrenceRule::new("FREQ=MONTHLY;BYDAY=1MO", "Europe/Amsterdam");
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn last_friday_of_month_passes_validation() {
        // Negative positional BYDAY (-1FR = last Friday) is supported.
        // The `-1` prefix selects the last occurrence of FR in the month.
        let rule = RecurrenceRule::new("FREQ=MONTHLY;BYDAY=-1FR", "Europe/Amsterdam");
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn yearly_rule_passes_validation() {
        // "Every October" — annual maintenance tasks like boiler service.
        // BYMONTH=10 constrains the YEARLY rule to October only.
        let rule = RecurrenceRule::new("FREQ=YEARLY;BYMONTH=10", "Europe/Amsterdam");
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn rule_with_count_passes_validation() {
        // COUNT-terminated rules are explicitly in scope.
        // COUNT=30 stops the recurrence after 30 instances regardless of the
        // materialisation horizon.
        let rule = RecurrenceRule::new("FREQ=DAILY;COUNT=30", "Europe/Amsterdam");
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn rule_with_until_passes_validation() {
        // UNTIL-terminated rules are explicitly in scope.
        // The UNTIL value is compared against each instance's UTC timestamp;
        // instances at or before UNTIL are included, instances after are not.
        let rule = RecurrenceRule::new("FREQ=WEEKLY;UNTIL=20261231T000000Z", "UTC");
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn byweekno_is_rejected() {
        // BYWEEKNO is explicitly out of scope — no household chore needs it.
        // Week numbers are ISO 8601 week-of-year and are timezone-sensitive in
        // ways that would complicate the UI without adding practical value.
        let rule = RecurrenceRule::new("FREQ=YEARLY;BYWEEKNO=10", "Europe/Amsterdam");
        let err = rule.validate().expect_err("BYWEEKNO must be rejected");
        assert!(
            matches!(err, RecurrenceError::UnsupportedFeature(_)),
            "expected UnsupportedFeature, got {err:?}"
        );
    }

    #[test]
    fn byyearday_is_rejected() {
        // BYYEARDAY is explicitly out of scope.
        let rule = RecurrenceRule::new("FREQ=YEARLY;BYYEARDAY=100", "Europe/Amsterdam");
        let err = rule.validate().expect_err("BYYEARDAY must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFeature(_)));
    }

    #[test]
    fn byhour_is_rejected() {
        // BYHOUR is explicitly out of scope — tasks are not hour-scheduled.
        let rule = RecurrenceRule::new("FREQ=DAILY;BYHOUR=8", "Europe/Amsterdam");
        let err = rule.validate().expect_err("BYHOUR must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFeature(_)));
    }

    #[test]
    fn byminute_is_rejected() {
        // BYMINUTE is explicitly out of scope.
        let rule = RecurrenceRule::new("FREQ=HOURLY;BYMINUTE=30", "Europe/Amsterdam");
        let err = rule.validate().expect_err("BYMINUTE must be rejected");
        // HOURLY frequency is also out of scope but BYMINUTE is checked first.
        assert!(matches!(
            err,
            RecurrenceError::UnsupportedFeature(_) | RecurrenceError::UnsupportedFrequency(_)
        ));
    }

    #[test]
    fn bysecond_is_rejected() {
        // BYSECOND is explicitly out of scope.
        let rule = RecurrenceRule::new("FREQ=MINUTELY;BYSECOND=0", "UTC");
        let err = rule.validate().expect_err("BYSECOND must be rejected");
        assert!(matches!(
            err,
            RecurrenceError::UnsupportedFeature(_) | RecurrenceError::UnsupportedFrequency(_)
        ));
    }

    #[test]
    fn bysetpos_is_rejected() {
        // BYSETPOS is explicitly out of scope — too complex for the UI.
        let rule = RecurrenceRule::new("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", "UTC");
        let err = rule.validate().expect_err("BYSETPOS must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFeature(_)));
    }

    #[test]
    fn wkst_mo_is_accepted() {
        // WKST=MO is the RFC 5545 default; explicitly accepting it is harmless
        // and avoids confusing clients who happen to include it.
        //
        // RFC 5545 defines WKST=MO as the default, so a rule that spells it out
        // explicitly must behave identically to one that omits it entirely.
        let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH;WKST=MO", "Europe/Amsterdam");
        assert!(
            rule.validate().is_ok(),
            "WKST=MO is the default and must be accepted"
        );
    }

    #[test]
    fn wkst_su_is_rejected() {
        // Non-default WKST shifts week-boundary semantics in ways the UI
        // does not expose or explain to the user.
        //
        // For example, WKST=SU changes how WEEKLY rules with INTERVAL>1 behave
        // near week boundaries — not meaningful for household chores.
        let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH;WKST=SU", "Europe/Amsterdam");
        let err = rule
            .validate()
            .expect_err("non-default WKST must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFeature(_)));
    }

    #[test]
    fn hourly_frequency_is_rejected() {
        // Sub-daily frequencies are not meaningful for household tasks.
        // HOURLY is rejected because: (1) no chore repeats hourly; (2) it would
        // interact with DST in complex ways that the materialiser doesn't handle.
        let rule = RecurrenceRule::new("FREQ=HOURLY", "UTC");
        let err = rule.validate().expect_err("HOURLY must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFrequency(_)));
    }

    #[test]
    fn minutely_frequency_is_rejected() {
        // MINUTELY is sub-daily and rejected by design — same rationale as HOURLY.
        let rule = RecurrenceRule::new("FREQ=MINUTELY", "UTC");
        let err = rule.validate().expect_err("MINUTELY must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFrequency(_)));
    }

    #[test]
    fn secondly_frequency_is_rejected() {
        // SECONDLY is sub-daily and rejected by design — same rationale as HOURLY.
        let rule = RecurrenceRule::new("FREQ=SECONDLY", "UTC");
        let err = rule.validate().expect_err("SECONDLY must be rejected");
        assert!(matches!(err, RecurrenceError::UnsupportedFrequency(_)));
    }

    #[test]
    fn error_message_names_unsupported_feature() {
        // The error message must identify the problematic feature so that the
        // service can return a 422 body that tells the client what to fix.
        //
        // The error type is `RecurrenceError::UnsupportedFeature(String)` where
        // the string is the feature name without its trailing `=` sign.
        // The service layer passes this string directly to the JSON error response
        // body; callers of the API see "unsupported RRULE feature: BYWEEKNO".
        let rule = RecurrenceRule::new("FREQ=YEARLY;BYWEEKNO=10", "UTC");
        let err = rule.validate().expect_err("must fail");
        // The error message should contain the feature name.
        let msg = err.to_string();
        assert!(
            msg.contains("BYWEEKNO"),
            "error message must name the feature, got: {msg}"
        );
    }
}
