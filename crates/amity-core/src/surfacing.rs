// surfacing.rs — the ranked "what's on today" query, as pure domain logic.
//
// Surfacing is the hard part of Amity, not storing (philosophy: "any database
// can remember; the skill is showing the right thing at the right moment").
// This module is where the system chooses to be *information* rather than
// pressure: an overdue task appears as a fact, never with a lateness count or
// an escalating colour (brief §3, §11).
//
// The brief (§6.4) describes surfacing as a single ranked query across Event,
// Task, Project milestones, and Thread prompts. It now ranks two kinds — Task
// and Event — behind a `SurfacedKind` enum that admits the rest later. Nothing
// here does I/O; the service layer gathers the candidates from storage and
// hands them in.
//
// The rule is deliberately kind-agnostic. Each candidate carries a `Liveness`
// (set by the caller) rather than a Task-specific status, so the same rule
// serves both kinds: a task is live while Open/Doing and settled once
// Done/Skipped; an event is live until it has passed. Ordering leads with
// all-day items (the wall-calendar banner), then by time, then priority, then a
// stable id tiebreak.
//
// The "surfaces on today" rule (ratified with the maintainer):
//   • a materialised recurring instance (or an event) surfaces on its own day;
//   • a one-shot task surfaces if its window intersects the day, OR it is still
//     open and its deadline has already passed (overdue-open);
//   • an event surfaces on its start date and is never "overdue" — it happens or
//     it has passed (a passed event simply drops off).
// A candidate with no temporal anchor makes no claim on today and stays quiet —
// the empty state is a designed state, and Amity errs toward silence.

// Serde derives let `SurfacedItem` cross the JSON API boundary unchanged.
use serde::{Deserialize, Serialize};
// `Date` is the day we surface for; `OffsetDateTime` is the salient instant.
use time::{Date, OffsetDateTime};

// `MemberId` is the displayed assignee; `Priority` orders items within a day.
use crate::ids::MemberId;
use crate::task::Priority;

// ─── SurfacedKind ───────────────────────────────────────────────────────────

/// The entity type a surfaced item came from.
///
/// `Task` and `Event` are live today; the remaining surfacable types named in
/// brief §6.4 — Project milestones, Thread prompts — join here later without a
/// change to the wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacedKind {
    /// A household Task (or a materialised instance of one).
    Task,
    /// A calendar Event (native, or a read-only external one).
    Event,
}

// ─── Liveness ───────────────────────────────────────────────────────────────

/// Whether a candidate is still worth surfacing, independent of its kind.
///
/// The caller maps its own status onto this: a task is `Live` while Open/Doing
/// and `Settled` once Done/Skipped; an event is `Live` until it has passed.
/// Keeping the rule on this flag — not on `TaskStatus` — is what lets one rule
/// serve every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Still open / upcoming — eligible to surface.
    Live,
    /// Resolved / finished — never surfaces.
    Settled,
}

// ─── Timing ─────────────────────────────────────────────────────────────────

/// The temporal anchor a candidate carries into the surfacing rule.
///
/// A recurring task instance and an event both use `Scheduled` (a fixed
/// occurrence that surfaces on its own day); a one-shot task uses `Window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    /// A fixed occurrence at a specific instant (recurring instance, or event).
    ///
    /// Surfaces only on its own day; never overdue. Missed prior instances are
    /// a `CompletionLog` concern the pure rule cannot see (see the module TODO).
    Scheduled(OffsetDateTime),

    /// A one-shot task's window: earliest meaningful start and latest due time.
    ///
    /// Either bound may be absent. `None`/`None` is a candidate with no temporal
    /// claim on any day; it never surfaces on Today.
    Window {
        /// Earliest time the task is meaningful to work on, if any.
        earliest_at: Option<OffsetDateTime>,
        /// Latest time the task should be done by, if any.
        due_by: Option<OffsetDateTime>,
    },
}

// ─── SurfaceCandidate ───────────────────────────────────────────────────────

/// One thing that *might* surface on a given day, before the rule is applied.
///
/// The service layer builds these from Task rows, their instances, and Event
/// rows; the pure rule decides which actually surface and in what order. The
/// type is decoupled from the concrete entities (the id is a plain string) so
/// the ranking logic depends on none of them.
#[derive(Debug, Clone)]
pub struct SurfaceCandidate {
    /// Which entity type this came from.
    pub kind: SurfacedKind,

    /// The source entity's id as a string (a task or event UUID).
    pub source_id: String,

    /// One-line title, shown verbatim in the Today view.
    pub title: String,

    /// Whether this is still worth surfacing (see [`Liveness`]).
    pub liveness: Liveness,

    /// The temporal anchor that decides whether this lands on the day.
    pub timing: Timing,

    /// True for an all-day event; such items lead the day and show "all day".
    pub all_day: bool,

    /// Soft importance rank, used only to break ties within a day. `None` for
    /// events, which carry no priority.
    pub priority: Option<Priority>,

    /// The member shown as responsible. May be `None`.
    pub current_assignee_id: Option<MemberId>,
}

// ─── SurfacedItem ───────────────────────────────────────────────────────────

/// A candidate that surfaced, in the uniform shape the Today view renders.
///
/// This is the wire type behind `GET /api/v1/surfacing/today`. It carries the
/// salient instant used for ordering and display, and an `overdue` flag the
/// frontend renders as plain information — never a red badge or a lateness
/// count (brief §3, §11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfacedItem {
    /// The entity type this item came from.
    pub kind: SurfacedKind,

    /// The source entity's id (a task or event UUID string).
    pub source_id: String,

    /// One-line title, shown verbatim.
    pub title: String,

    /// The salient instant: the scheduled time, or the due/earliest time.
    ///
    /// Used for ordering within the day and for display ("this morning").
    pub at: OffsetDateTime,

    /// True when the item surfaced because it is open past its deadline.
    ///
    /// Only tasks are ever overdue; events are never flagged this way.
    pub overdue: bool,

    /// True for an all-day event; the frontend shows "all day" instead of a time.
    pub all_day: bool,

    /// Soft importance rank, carried through for display and secondary ordering.
    pub priority: Option<Priority>,

    /// The member shown as responsible, if any.
    pub current_assignee_id: Option<MemberId>,
}

// ─── SurfacingConfig ────────────────────────────────────────────────────────

/// Inputs that shape *what* surfaces, beyond the raw temporal rule.
///
/// Only `member_filter` is live. Quiet hours and Presence-based filtering are
/// named here as the seam for later work — they need the member-preference and
/// Presence entities, which do not exist yet.
#[derive(Debug, Clone, Default)]
pub struct SurfacingConfig {
    /// When set, keep only items whose current assignee is this member.
    ///
    /// This is the "my today" filter. `None` surfaces the whole household's day.
    /// With only the placeholder member today it is mostly forward-looking, but
    /// the logic is real and tested — it is what a per-person hub view will use
    /// once members exist. Applied after the temporal rule, so it narrows an
    /// already-surfaced day rather than changing what counts as "on today".
    pub member_filter: Option<MemberId>,
    // TODO(quiet-hours): suppress non-critical items during a member's quiet
    // hours once member preferences exist (brief §11.3).
    // TODO(presence): drop general household items for members who are `away`
    // or `with_other_parent` once the Presence entity lands (brief §6.5).
}

// ─── SurfacingResult ────────────────────────────────────────────────────────

/// The outcome of ranking a day's candidates.
///
/// `has_surfaced` is the designed-empty-state signal: when it is `false` the
/// Today view shows a calm "nothing today" and means it, rather than an error
/// or a spinner (brief §3, §11.5). It is simply whether `items` is non-empty,
/// carried explicitly so the wire contract states the empty state as a fact the
/// client can key off, not something it has to infer from an array length.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacingResult {
    /// The items that surfaced, ordered for the Today view.
    pub items: Vec<SurfacedItem>,

    /// Whether anything crossed the "surface today" threshold at all.
    pub has_surfaced: bool,
}

// ─── The ranking function ───────────────────────────────────────────────────

/// Rank the candidates that surface on `date`, given the current instant `now`.
///
/// The pipeline has three filter stages and one sort:
///   1. drop settled candidates (a task that is Done/Skipped never surfaces);
///   2. apply the temporal rule per candidate (`evaluate` → `None` off-today);
///   3. apply the per-person `member_filter`, if the config sets one;
///   4. order the survivors — all-day first, then by salient time ascending,
///      then priority descending, then source id for a deterministic tiebreak.
///
/// Every stage is a pure predicate or mapping, so the whole function is a pure
/// transform of its inputs. Returns the ordered items plus the empty-state flag
/// (`has_surfaced`), which the Today view keys its calm "nothing today" state on.
#[must_use]
pub fn rank_today(
    candidates: Vec<SurfaceCandidate>,
    date: Date,
    now: OffsetDateTime,
    config: &SurfacingConfig,
) -> SurfacingResult {
    // Pipeline: drop settled candidates, keep only those that surface on the
    // day, then apply the per-person filter. Each stage is a pure predicate.
    let mut items: Vec<SurfacedItem> = candidates
        .into_iter()
        // Settled (Done/Skipped, or otherwise finished) candidates never surface.
        .filter(|c| c.liveness == Liveness::Live)
        // Evaluate the temporal rule; `None` means "not on today".
        .filter_map(|c| evaluate(c, date, now))
        // "My today": keep only the chosen member's items when a filter is set.
        .filter(|item| member_matches(item, config))
        .collect();

    // Order: all-day items lead the day (the wall-calendar banner), then by time
    // proximity, then priority descending, then source id for determinism.
    //
    // Worked example for one day, the four keys applied in turn:
    //   1. all-day "king's day"      — all-day, so it leads regardless of time
    //   2. overdue "renew passport"  — 09:00 yesterday, the earliest salient time
    //   3. "school run"              — 08:00 today
    //   4. "call plumber"            — 17:00 today
    // Two items sharing an instant fall back to priority, then to source id, so
    // the output is fully deterministic.
    items.sort_by(|a, b| {
        // `true` (all-day) must sort before `false`, hence b compared to a.
        b.all_day
            .cmp(&a.all_day)
            .then(a.at.cmp(&b.at))
            .then_with(|| priority_rank(b).cmp(&priority_rank(a)))
            .then_with(|| a.source_id.cmp(&b.source_id))
    });

    // The empty-state signal: false → the Today view says "nothing today".
    let has_surfaced = !items.is_empty();
    SurfacingResult {
        items,
        has_surfaced,
    }
}

// ─── Private rule helpers ───────────────────────────────────────────────────

/// Decide whether a live candidate surfaces on `date`, and build its item.
///
/// Returns `None` when the candidate makes no claim on the day. On success the
/// salient instant and the overdue flag are already resolved by `timing_on_day`;
/// this function just projects the candidate into the wire `SurfacedItem`,
/// moving its owned fields (id, title) rather than cloning them.
fn evaluate(candidate: SurfaceCandidate, date: Date, now: OffsetDateTime) -> Option<SurfacedItem> {
    // `timing_on_day` carries all the rule logic; `?` short-circuits to None
    // when the candidate makes no claim on the day.
    let (at, overdue) = timing_on_day(candidate.timing, date, now)?;
    // Project the surviving candidate into the wire item. Copy fields carry over
    // directly; the salient time and overdue flag come from the rule above.
    Some(SurfacedItem {
        // Entity type — Task or Event.
        kind: candidate.kind,
        // Move the id/title out of the candidate — no clone needed.
        source_id: candidate.source_id,
        title: candidate.title,
        // The instant used for ordering and display.
        at,
        // Whether this surfaced via the overdue-open path (tasks only).
        overdue,
        // All-day flag drives the "all day" label and the lead-the-day ordering.
        all_day: candidate.all_day,
        // Soft rank, carried through for the tiebreak and display.
        priority: candidate.priority,
        // The member shown as responsible, if any.
        current_assignee_id: candidate.current_assignee_id,
    })
}

/// Apply the ratified "surfaces on today" rule to one temporal anchor.
///
/// Returns `Some((salient_time, overdue))` if it surfaces, else `None`. The two
/// timing shapes are handled differently:
///   • `Scheduled` (a recurring instance or an event) surfaces only when its own
///     day equals `date`, and is never overdue — a fixed occurrence happens or
///     it has passed.
///   • `Window` (a one-shot task) surfaces when open-and-overdue, or when today
///     falls inside its `[earliest, due]` window; an undated window makes no
///     claim on any day.
fn timing_on_day(
    timing: Timing,
    date: Date,
    now: OffsetDateTime,
) -> Option<(OffsetDateTime, bool)> {
    match timing {
        // A fixed occurrence (recurring instance or event) surfaces on its day.
        Timing::Scheduled(at) => (at.date() == date).then_some((at, false)),

        Timing::Window {
            earliest_at,
            due_by,
        } => match due_by {
            // Deadline already passed → surfaces overdue; the salient time is due.
            // Only reached for live candidates (settled ones were filtered out).
            Some(due) if due < now => Some((due, true)),
            // Deadline still ahead and today is in-window → surfaces on time.
            Some(due) if within_window(earliest_at, Some(due), date) => Some((due, false)),
            // Deadline still ahead but today is outside the window → no claim.
            Some(_) => None,
            // No deadline: it surfaces once, on the day its window opens — an
            // undated candidate with no start makes no claim on any day.
            None => match earliest_at {
                Some(start) if start.date() == date => Some((start, false)),
                _ => None,
            },
        },
    }
}

/// Whether `date` falls within `[earliest, due]`, treating `None` bounds as open.
///
/// Compared at day granularity: a task is "on today" for the whole of a day its
/// window overlaps, not only at an exact instant. So a task with no `earliest`
/// and a `due` three days out is "on today" every day until then — the window is
/// the span it is workable in, and surfacing shows it for the whole span. An
/// absent bound is open on that side (no earliest → workable from the start;
/// no due → no deadline at all).
fn within_window(
    earliest: Option<OffsetDateTime>,
    due: Option<OffsetDateTime>,
    date: Date,
) -> bool {
    // Past (or at) the opening bound — an absent earliest means "always open".
    let after_start = earliest.is_none_or(|e| e.date() <= date);
    // On or before the closing bound — an absent due means "no deadline".
    let before_end = due.is_none_or(|d| date <= d.date());
    // The day is in-window only when both bounds are satisfied.
    after_start && before_end
}

/// Numeric priority for ordering; unset priority ranks lowest.
///
/// Only reached as the third sort key, after all-day and time, so it settles
/// items that share the same salient instant. Events always rank 0 here.
fn priority_rank(item: &SurfacedItem) -> u8 {
    // No priority set → 0, which sorts behind any explicit 1..=5 rank.
    item.priority.map_or(0, Priority::value)
}

/// Keep an item when no member filter is set, or when it belongs to that member.
///
/// This is the "my today" narrowing. When a member filter is set, an item with
/// no assignee is dropped — it belongs to nobody's personal day.
fn member_matches(item: &SurfacedItem, config: &SurfacingConfig) -> bool {
    // No filter → keep everything; a filter → keep only that member's items.
    // Unassigned items are dropped under a filter (they are nobody's "my today").
    config
        .member_filter
        .is_none_or(|m| item.current_assignee_id == Some(m))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Bring the module's public API and private helpers into test scope.
    use super::*;
    // `date!`/`datetime!` build the fixed day and the salient instants used below.
    use time::macros::{date, datetime};

    // ── Fixtures ─────────────────────────────────────────────────────────────
    // One fixed day, one fixed `now`, and named instants around it, so each test
    // reads as a small scenario in wall-clock time. The day's timeline:
    //
    //   yesterday 09:00  — a deadline that has already passed (overdue path)
    //   today    00:00   — the all-day banner anchor
    //   today    08:00   — morning():  a scheduled time before `now`
    //   today    12:00   — now():      the current instant
    //   today    15:00   — afternoon(): later the same day
    //   today    17:00   — evening():  due later today, still in the future
    //   tomorrow 08:00   — the next day, which makes no claim on today
    //
    // Every fixture below returns one of these fixed instants or a fixed member,
    // so the whole suite is deterministic regardless of the real wall clock.

    /// The placeholder member (matches the UUID inserted by migration 0001).
    fn member() -> MemberId {
        // Parsing this constant is infallible; unwrap keeps the fixture terse.
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    /// A second, distinct member, used only by the per-person filter test.
    fn other_member() -> MemberId {
        // A different fixed UUID so the "my today" filter has someone to exclude.
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000002").unwrap())
    }

    /// The single day every test surfaces for; fixing it keeps the suite pure.
    fn today() -> Date {
        // 2026-07-24 is the project's "today" throughout these fixtures.
        date!(2026 - 07 - 24)
    }

    /// "Now" — midday on the surfaced day, so a morning deadline reads as overdue.
    fn now() -> OffsetDateTime {
        // Midday sits after the morning instant and before the evening one.
        datetime!(2026-07-24 12:00:00 UTC)
    }

    /// This morning (08:00 today) — before `now`, a common scheduled time.
    fn morning() -> OffsetDateTime {
        // Used for on-time same-day instances and events.
        datetime!(2026-07-24 08:00:00 UTC)
    }

    /// This afternoon (15:00 today) — after `now`, later in the same day.
    fn afternoon() -> OffsetDateTime {
        // Used to prove time ordering within the day.
        // Sorts after `morning` but before `evening`.
        datetime!(2026-07-24 15:00:00 UTC)
    }

    /// This evening (17:00 today) — a deadline that is today but still future.
    fn evening() -> OffsetDateTime {
        // Due-later-today: on Today, but not yet overdue.
        // Later than `now` (12:00), so a task due here is not overdue.
        datetime!(2026-07-24 17:00:00 UTC)
    }

    /// Yesterday morning (09:00) — a deadline that has already passed.
    fn yesterday() -> OffsetDateTime {
        // Overdue relative to `now`; the overdue-open path uses this.
        // A prior-day instant, so a scheduled occurrence here is off-today.
        datetime!(2026-07-23 09:00:00 UTC)
    }

    /// Tomorrow morning (08:00) — a scheduled time on the wrong day.
    fn tomorrow() -> OffsetDateTime {
        // Proves a future occurrence makes no claim on today.
        // A next-day instant; `Scheduled` here surfaces on tomorrow, not today.
        datetime!(2026-07-25 08:00:00 UTC)
    }

    /// A `Scheduled` timing at `dt` — a recurring instance or an event.
    fn sched(dt: OffsetDateTime) -> Timing {
        // Wrap the instant in the Scheduled variant for terse setup.
        // Scheduled timings surface only on their own day, never overdue.
        Timing::Scheduled(dt)
    }

    /// A one-shot `Window` timing with optional `earliest`/`due` bounds.
    fn win(earliest: Option<OffsetDateTime>, due: Option<OffsetDateTime>) -> Timing {
        // The one-shot path: an open-ended window the rule intersects with today.
        Timing::Window {
            // Earliest meaningful start (None = always open before the deadline).
            earliest_at: earliest,
            // Latest due time (None = no deadline at all).
            due_by: due,
        }
    }

    /// A fresh, distinct id string for a candidate (uniqueness only).
    fn some_id() -> String {
        // A time-ordered UUID keeps ids distinct without a shared counter.
        // The value is irrelevant to the rule; only its uniqueness matters.
        uuid::Uuid::now_v7().to_string()
    }

    /// Build a Task candidate with the given liveness and timing.
    fn task(title: &str, liveness: Liveness, timing: Timing) -> SurfaceCandidate {
        // Tasks are never all-day; priority defaults to None; assigned to member.
        SurfaceCandidate {
            // A task-kind candidate.
            kind: SurfacedKind::Task,
            // A distinct id per candidate.
            source_id: some_id(),
            // The caller's display title.
            title: title.to_owned(),
            // The caller's liveness under test.
            liveness,
            // The caller's temporal anchor under test.
            timing,
            // Tasks never surface as all-day.
            all_day: false,
            // No priority unless a test sets one.
            priority: None,
            // Assigned to the placeholder member so the filter test has a subject.
            current_assignee_id: Some(member()),
        }
    }

    /// Build an Event candidate at `timing`; `all_day` marks a banner event.
    fn event(title: &str, timing: Timing, all_day: bool) -> SurfaceCandidate {
        // Events are always Live, carry no priority, and have no assignee here.
        SurfaceCandidate {
            // An event-kind candidate.
            kind: SurfacedKind::Event,
            // A distinct id per candidate.
            source_id: some_id(),
            // The caller's display title.
            title: title.to_owned(),
            // Events are live until they have passed.
            liveness: Liveness::Live,
            // The caller's temporal anchor.
            timing,
            // Whether this is an all-day banner event.
            all_day,
            // Events carry no priority.
            priority: None,
            // No assignee on events in these tests.
            current_assignee_id: None,
        }
    }

    /// Rank a batch against the fixed day/now with default config.
    fn rank(candidates: Vec<SurfaceCandidate>) -> SurfacingResult {
        // Every test funnels through this call so the day and now stay consistent.
        rank_today(candidates, today(), now(), &SurfacingConfig::default())
    }

    /// The surfaced titles, in order — the shape the ordering tests assert on.
    fn titles(result: &SurfacingResult) -> Vec<&str> {
        // Collect just the titles so ordering assertions stay readable.
        // Borrowed &str slices — the ordering is what the assertions check.
        result.items.iter().map(|i| i.title.as_str()).collect()
    }

    // ── Threshold: what surfaces on the day ──────────────────────────────────

    #[test]
    fn empty_input_surfaces_nothing() {
        // The designed empty state is a real result, not an error or a spinner:
        // with no candidates the Today view says "nothing today" and means it.
        let result = rank(vec![]);
        // Nothing comes back.
        assert!(result.items.is_empty());
        // And the empty-state flag stays false, which the frontend keys off.
        assert!(
            !result.has_surfaced,
            "an empty day must not report surfacing"
        );
    }

    #[test]
    fn threshold_rule_decides_what_surfaces() {
        // The whole "on today" rule as a table of cases, one candidate each:
        //   (label, liveness, timing, expected-surfaces, expected-overdue).
        // Each row walks one branch of the rule so the boundary reads at a glance;
        // a settled candidate is filtered out before the temporal rule even runs.
        let cases: [(&str, Liveness, Timing, bool, bool); 8] = [
            // A scheduled instance on today surfaces, not overdue.
            (
                "scheduled today",
                Liveness::Live,
                sched(morning()),
                true,
                false,
            ),
            // Tomorrow's instance makes no claim on today.
            (
                "scheduled tomorrow",
                Liveness::Live,
                sched(tomorrow()),
                false,
                false,
            ),
            // A one-shot due later today surfaces, not overdue.
            (
                "due later today",
                Liveness::Live,
                win(None, Some(evening())),
                true,
                false,
            ),
            // Open past yesterday's deadline → surfaces as information (overdue).
            (
                "overdue and live",
                Liveness::Live,
                win(None, Some(yesterday())),
                true,
                true,
            ),
            // Same past deadline but settled → never surfaces.
            (
                "overdue but settled",
                Liveness::Settled,
                win(None, Some(yesterday())),
                false,
                false,
            ),
            // Today inside [earliest, due] → surfaces, not overdue.
            (
                "inside window",
                Liveness::Live,
                win(
                    Some(datetime!(2026-07-23 00:00:00 UTC)),
                    Some(datetime!(2026-07-25 23:00:00 UTC)),
                ),
                true,
                false,
            ),
            // Window opens next week → nothing today.
            (
                "before window opens",
                Liveness::Live,
                win(
                    Some(datetime!(2026-07-28 00:00:00 UTC)),
                    Some(datetime!(2026-07-30 00:00:00 UTC)),
                ),
                false,
                false,
            ),
            // No dates at all → makes no claim on today.
            ("undated", Liveness::Live, win(None, None), false, false),
        ];
        // Run every case through a single-candidate day and check the outcome.
        for (what, liveness, timing, surfaces, overdue) in cases {
            // Build and rank a one-candidate day for this case (kind irrelevant).
            let result = rank(vec![task(what, liveness, timing)]);
            // The surfaced count and the empty-state flag must match expectation.
            assert_eq!(
                result.items.len(),
                usize::from(surfaces),
                "surfaces: {what}"
            );
            assert_eq!(result.has_surfaced, surfaces, "flag: {what}");
            // When it surfaces, the overdue flag must match too.
            if surfaces {
                assert_eq!(result.items[0].overdue, overdue, "overdue: {what}");
            }
        }
    }

    // ── Events ───────────────────────────────────────────────────────────────

    #[test]
    fn event_surfaces_on_its_day_and_is_never_overdue() {
        // An event on today surfaces as an Event-kind item; unlike a task it is
        // never flagged overdue — it happens or it has passed. This is the base
        // case that proves the mixed-type seam is real, not just declared.
        // A timed morning event.
        let e = event("school run", sched(morning()), false);
        // Rank the single-event day.
        let result = rank(vec![e]);
        // Exactly one item comes back.
        assert_eq!(result.items.len(), 1);
        // It is tagged Event, not Task.
        assert_eq!(result.items[0].kind, SurfacedKind::Event);
        // And it is never overdue — events are not nagged like open tasks.
        assert!(!result.items[0].overdue);
    }

    #[test]
    fn all_day_event_leads_the_day() {
        // An all-day event sorts ahead of a timed morning event and even ahead of
        // an overdue task (yesterday) — like the banner atop a wall calendar, the
        // day's context reads before its clock-timed entries.
        // An all-day banner event anchored at local midnight.
        let banner = event(
            "king's day",
            sched(datetime!(2026-07-24 00:00:00 UTC)),
            true,
        );
        // A timed morning event (08:00).
        let timed = event("dentist", sched(morning()), false);
        // An overdue but still-live task (yesterday's deadline).
        let overdue = task(
            "renew passport",
            Liveness::Live,
            win(None, Some(yesterday())),
        );
        // Pass out of order to prove the sort, not the input order, decides.
        let result = rank(vec![timed, overdue, banner]);
        // The all-day banner leads, ahead of the timed and overdue items.
        assert_eq!(titles(&result)[0], "king's day");
    }

    #[test]
    fn tasks_and_events_rank_together_by_time() {
        // A task and an event on the same day interleave purely by salient time —
        // the whole point of the mixed-type seam.
        // An event at 08:00.
        let morning_event = event("school run", sched(morning()), false);
        // A task due at 17:00.
        let noon_task = task("call plumber", Liveness::Live, win(None, Some(evening())));
        // Rank the mixed pair.
        let result = rank(vec![noon_task, morning_event]);
        // The 08:00 event precedes the 17:00 task deadline, regardless of kind.
        assert_eq!(titles(&result), vec!["school run", "call plumber"]);
    }

    // ── Ordering and filtering ───────────────────────────────────────────────

    #[test]
    fn items_are_ordered_by_time_then_priority() {
        // Ordering is by time proximity first (after the all-day lead): an overdue
        // item (yesterday) sorts ahead of the morning instance, then the afternoon.
        // Priority is untouched here — it only breaks exact ties (next test).
        // Yesterday's overdue deadline: the earliest salient time.
        let overdue = task(
            "overdue thing",
            Liveness::Live,
            win(None, Some(yesterday())),
        );
        // This morning's instance: middle.
        let morning_item = task("morning thing", Liveness::Live, sched(morning()));
        // This afternoon's instance: latest.
        let afternoon_item = task("afternoon thing", Liveness::Live, sched(afternoon()));
        // Pass them out of order to prove the sort — not the input — decides.
        let result = rank(vec![afternoon_item, morning_item, overdue]);
        // Earliest salient time first, all the way down.
        assert_eq!(
            titles(&result),
            vec!["overdue thing", "morning thing", "afternoon thing"]
        );
    }

    #[test]
    fn priority_breaks_ties_at_the_same_time() {
        // Two items at the identical instant: priority is the tiebreak, high first.
        // This is the only place priority affects order.
        // Both items share this instant.
        let at = morning();
        // The low-priority one (rank 2).
        let mut low = task("low", Liveness::Live, sched(at));
        low.priority = Some(Priority::new(2).unwrap());
        // The high-priority one (rank 5).
        let mut high = task("high", Liveness::Live, sched(at));
        high.priority = Some(Priority::new(5).unwrap());
        // Rank the tied pair.
        let result = rank(vec![low, high]);
        // Higher priority wins the tie.
        assert_eq!(titles(&result), vec!["high", "low"]);
    }

    #[test]
    fn member_filter_keeps_only_that_members_items() {
        // The "my today" filter narrows the day to one member: only items whose
        // current assignee matches survive; everyone else's work is excluded.
        // My task (assigned to the placeholder member by default).
        let mine = task("my task", Liveness::Live, sched(morning()));
        // A second item reassigned to someone else, which the filter must drop.
        let mut theirs = task("their task", Liveness::Live, sched(afternoon()));
        theirs.current_assignee_id = Some(other_member());
        // Filter the day down to the placeholder member.
        let config = SurfacingConfig {
            // Only this member's items should survive.
            member_filter: Some(member()),
        };
        // Rank with the filter applied.
        let result = rank_today(vec![mine, theirs], today(), now(), &config);
        // Only the placeholder member's task survives.
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].title, "my task");
    }
}
