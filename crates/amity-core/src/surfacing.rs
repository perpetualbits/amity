// surfacing.rs — the ranked "what's on today" query, as pure domain logic.
//
// Surfacing is the hard part of Amity, not storing (philosophy: "any database
// can remember; the skill is showing the right thing at the right moment").
// This module is where the system chooses to be *information* rather than
// pressure: an overdue task appears as a fact, never with a lateness count or
// an escalating colour (brief §3, §11).
//
// The brief (§6.4) describes surfacing as a single ranked query across Event,
// Task, Project milestones, and Thread prompts. Only Task exists today, so this
// module ranks over Tasks alone — behind a `SurfacedKind` enum that is the seam
// for the other entity types. Nothing here does I/O; the service layer gathers
// the candidates from storage and hands them in.
//
// The "surfaces on today" rule (ratified with the maintainer):
//   • a materialised recurring instance surfaces on its scheduled day; and
//   • a one-shot task surfaces if its window intersects the day, OR it is still
//     open and its deadline has already passed (overdue-open).
// A task with no temporal anchor at all makes no claim on today and stays quiet
// — the empty state is a designed state, and Amity errs toward silence.
//
// Ordering, once the set is chosen, is by time proximity first (the earliest
// salient instant leads), then by priority as a tiebreak, then by source id so
// the order is fully deterministic. Priority never lets an important task jump
// ahead of an earlier one — it only settles exact ties.
//
// `SurfacingConfig` carries the levers that shape *what* surfaces beyond the raw
// temporal rule. Today only the per-person "my today" filter is live; quiet
// hours and Presence-based filtering are named as seams for when the member and
// Presence entities exist.

// Serde derives let `SurfacedItem` cross the JSON API boundary unchanged.
use serde::{Deserialize, Serialize};
// `Date` is the day we surface for; `OffsetDateTime` is the salient instant.
use time::{Date, OffsetDateTime};

// Typed IDs for the source entity and the displayed assignee.
use crate::ids::{MemberId, TaskId};
// Priority orders items within a day; TaskStatus decides what is still live.
use crate::task::{Priority, TaskStatus};

// ─── SurfacedKind ───────────────────────────────────────────────────────────

/// The entity type a surfaced item came from.
///
/// Only `Task` exists today. The enum is the honest seam for the other
/// surfacable types named in brief §6.4 — Event, Project milestones, Thread
/// prompts — so the Today view renders a mixed-type list from day one and the
/// wire shape does not change when they arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacedKind {
    /// The item is a household Task (or a materialised instance of one).
    Task,
}

// ─── Timing ─────────────────────────────────────────────────────────────────

/// The temporal anchor a candidate carries into the surfacing rule.
///
/// A recurring task contributes `Scheduled` instances (one per materialised
/// day); a one-shot task contributes a single `Window`. Keeping these distinct
/// lets the rule treat a fixed occurrence differently from an open window
/// without the caller having to pre-flatten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    /// A materialised recurring instance scheduled for a specific instant.
    ///
    /// It surfaces only on its own day — missed prior instances are a
    /// `CompletionLog` concern the pure rule cannot see (see the module TODO).
    Scheduled(OffsetDateTime),

    /// A one-shot task's window: earliest meaningful start and latest due time.
    ///
    /// Either bound may be absent. `None`/`None` is a task with no temporal
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
/// The service layer builds these by joining Task rows with their materialised
/// instances; the pure rule decides which ones actually surface and in what
/// order. This type is deliberately decoupled from both `Task` and the storage
/// `TaskInstance` so the ranking logic depends on neither.
#[derive(Debug, Clone)]
pub struct SurfaceCandidate {
    /// Which entity type this came from. `Task` for now.
    pub kind: SurfacedKind,

    /// The source Task's id (the parent task, for recurring instances).
    pub source_id: TaskId,

    /// One-line title, shown verbatim in the Today view.
    pub title: String,

    /// The source task's lifecycle status. `Done`/`Skipped` never surface.
    pub status: TaskStatus,

    /// The temporal anchor that decides whether this lands on the day.
    pub timing: Timing,

    /// Soft importance rank, used only to break ties within a day.
    pub priority: Option<Priority>,

    /// The member shown as responsible. May be `None` (no default assignee).
    pub current_assignee_id: Option<MemberId>,
}

// ─── SurfacedItem ───────────────────────────────────────────────────────────

/// A candidate that surfaced, in the uniform shape the Today view renders.
///
/// This is the wire type returned by `GET /api/v1/surfacing/today`. It carries
/// the salient instant used for ordering and display, and an `overdue` flag so
/// the frontend can say "due earlier" as information — never a red badge or a
/// count of how late the task is (brief §3, §11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfacedItem {
    /// The entity type this item came from.
    pub kind: SurfacedKind,

    /// The source Task's id (parent task for a recurring instance).
    pub source_id: TaskId,

    /// One-line title, shown verbatim.
    pub title: String,

    /// The source task's lifecycle status (always `Open` or `Doing` here).
    pub status: TaskStatus,

    /// The salient instant: the scheduled time, or the due/earliest time.
    ///
    /// Used both for ordering within the day and for display ("this morning").
    pub at: OffsetDateTime,

    /// True when the item surfaced because it is open past its deadline.
    ///
    /// The frontend renders this as plain information. Amity does not shade
    /// overdue items in a threatening colour or accumulate guilt-debt.
    pub overdue: bool,

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
    /// This is the "my today" filter. With the placeholder member it is mostly
    /// forward-looking, but the logic is real and tested. `None` surfaces the
    /// whole household's day.
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
/// or a spinner (brief §3, §11.5).
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
/// Applies the ratified rule (module docs), filters by `config`, and orders the
/// survivors by salient time ascending, then priority descending, then source
/// id for a deterministic tiebreak. Returns the ordered items plus the
/// empty-state flag.
#[must_use]
pub fn rank_today(
    candidates: Vec<SurfaceCandidate>,
    date: Date,
    now: OffsetDateTime,
    config: &SurfacingConfig,
) -> SurfacingResult {
    // Pipeline: drop resolved tasks, keep only those that surface on the day,
    // then apply the per-person filter. Each stage is a pure predicate/mapping.
    let mut items: Vec<SurfacedItem> = candidates
        .into_iter()
        // Done/Skipped tasks are settled; they never surface.
        .filter(|c| is_live(c.status))
        // Evaluate the temporal rule; `None` means "not on today".
        .filter_map(|c| evaluate(c, date, now))
        // "My today": keep only the chosen member's items when a filter is set.
        .filter(|item| member_matches(item, config))
        .collect();

    // Order by time proximity first (earliest salient time wins), then by
    // priority descending, then by source id so the ordering is deterministic.
    items.sort_by(|a, b| {
        // Primary key: the earlier salient instant comes first.
        a.at.cmp(&b.at)
            // Tiebreak 1: higher priority first (note the reversed operands).
            .then_with(|| priority_rank(b).cmp(&priority_rank(a)))
            // Tiebreak 2: source id, purely so equal items sort deterministically.
            .then_with(|| a.source_id.0.cmp(&b.source_id.0))
    });

    // The empty-state signal: false → the Today view says "nothing today".
    let has_surfaced = !items.is_empty();
    SurfacingResult {
        items,
        has_surfaced,
    }
}

// ─── Private rule helpers ───────────────────────────────────────────────────

/// A task is live (can still surface) only while `Open` or `Doing`.
///
/// `Done` and `Skipped` are terminal — surfacing them would turn Today into a
/// record of the past rather than a view of what still wants attention.
fn is_live(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Open | TaskStatus::Doing)
}

/// Decide whether a live candidate surfaces on `date`, and build its item.
///
/// Returns `None` when the candidate makes no claim on the day. On success the
/// salient instant and the overdue flag are already resolved.
fn evaluate(candidate: SurfaceCandidate, date: Date, now: OffsetDateTime) -> Option<SurfacedItem> {
    // `timing_on_day` carries all the rule logic; the rest is field-moving.
    // `?` short-circuits to `None` when the candidate makes no claim on the day.
    let (at, overdue) = timing_on_day(candidate.timing, date, now)?;
    // Project the candidate into the wire item. Copy fields carry over directly;
    // the salient time and overdue flag come from the rule above.
    Some(SurfacedItem {
        // Entity type — `Task` today, the seam for others later.
        kind: candidate.kind,
        // The parent task, so the frontend can act on it (complete, reassign).
        source_id: candidate.source_id,
        // Move the title out of the candidate — no clone needed.
        title: candidate.title,
        // Status is always live here (Done/Skipped were filtered upstream).
        status: candidate.status,
        // The instant used for ordering and display.
        at,
        // Whether this surfaced via the overdue-open path.
        overdue,
        // Soft rank, carried through for the ordering tiebreak and display.
        priority: candidate.priority,
        // The member shown as responsible, if any.
        current_assignee_id: candidate.current_assignee_id,
    })
}

/// Apply the ratified "surfaces on today" rule to one temporal anchor.
///
/// Returns `Some((salient_time, overdue))` if it surfaces, else `None`.
fn timing_on_day(
    timing: Timing,
    date: Date,
    now: OffsetDateTime,
) -> Option<(OffsetDateTime, bool)> {
    match timing {
        // A materialised recurring instance surfaces only on its own day.
        Timing::Scheduled(at) => (at.date() == date).then_some((at, false)),

        Timing::Window {
            earliest_at,
            due_by,
        } => match due_by {
            // With a deadline: overdue-open takes precedence, else it must fall
            // inside its window. Overdue is only reached for live tasks because
            // resolved ones were filtered out before `evaluate`.
            //
            // Deadline already passed → surfaces overdue, salient time is the due.
            Some(due) if due < now => Some((due, true)),
            // Deadline still ahead and today is in-window → surfaces on time.
            Some(due) if within_window(earliest_at, Some(due), date) => Some((due, false)),
            // Deadline still ahead but today is outside the window → no claim.
            Some(_) => None,
            // No deadline: it surfaces once, on the day its window opens — an
            // undated task with no start makes no claim on any day.
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
/// window overlaps, not only at an exact instant.
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
fn priority_rank(item: &SurfacedItem) -> u8 {
    // No priority set → 0, which sorts behind any explicit 1..=5 rank.
    item.priority.map_or(0, Priority::value)
}

/// Keep an item when no member filter is set, or when it belongs to that member.
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
    // The suite is deterministic: one fixed day, one fixed `now`, and named
    // instants around them so each test reads as a small scenario in wall-clock
    // time rather than a wall of `datetime!` literals.

    /// The placeholder member (matches the UUID inserted by migration 0001).
    fn member() -> MemberId {
        // Parsing this constant is infallible; unwrap keeps the fixture terse.
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    /// A second, distinct member — only the per-person filter test needs it.
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
        // Used for on-time same-day instances.
        datetime!(2026-07-24 08:00:00 UTC)
    }

    /// This afternoon (15:00 today) — after `now`, later in the same day.
    fn afternoon() -> OffsetDateTime {
        // Used to prove time ordering within the day.
        datetime!(2026-07-24 15:00:00 UTC)
    }

    /// This evening (17:00 today) — a deadline that is today but still future.
    fn evening() -> OffsetDateTime {
        // Due-later-today: on Today, but not yet overdue.
        datetime!(2026-07-24 17:00:00 UTC)
    }

    /// Yesterday morning (09:00) — a deadline that has already passed.
    fn yesterday() -> OffsetDateTime {
        // Overdue relative to `now`; the overdue-open path uses this.
        datetime!(2026-07-23 09:00:00 UTC)
    }

    /// Tomorrow morning (08:00) — a scheduled time on the wrong day.
    fn tomorrow() -> OffsetDateTime {
        // Proves a future instance makes no claim on today.
        datetime!(2026-07-25 08:00:00 UTC)
    }

    /// A recurring instance scheduled at `dt` — terse `Timing::Scheduled` sugar.
    fn sched(dt: OffsetDateTime) -> Timing {
        // The recurring path: one materialised occurrence at a fixed instant.
        Timing::Scheduled(dt)
    }

    /// A one-shot window with optional `earliest`/`due` bounds.
    fn win(earliest: Option<OffsetDateTime>, due: Option<OffsetDateTime>) -> Timing {
        // The one-shot path: an open-ended window the rule intersects with today.
        Timing::Window {
            earliest_at: earliest,
            due_by: due,
        }
    }

    /// Build a candidate: `Task` kind, assigned to the placeholder member.
    fn candidate(title: &str, status: TaskStatus, timing: Timing) -> SurfaceCandidate {
        // Priority defaults to None; the tie-break test overrides it explicitly.
        SurfaceCandidate {
            // Everything is a Task in this task's scope.
            kind: SurfacedKind::Task,
            // A fresh id per candidate so the deterministic-tiebreak path is real.
            source_id: TaskId::new(),
            // The caller supplies the display title.
            title: title.to_owned(),
            // The caller supplies the lifecycle status under test.
            status,
            // The caller supplies the temporal anchor under test.
            timing,
            // No priority by default; the tie-break test sets one.
            priority: None,
            // Assigned to the placeholder member so the filter test has a subject.
            current_assignee_id: Some(member()),
        }
    }

    /// Rank a batch against the fixed day/now with default config — the common case.
    fn rank(candidates: Vec<SurfaceCandidate>) -> SurfacingResult {
        // Every test funnels through this call so the day and now stay consistent.
        rank_today(candidates, today(), now(), &SurfacingConfig::default())
    }

    /// The surfaced titles, in order — the shape the ordering tests assert on.
    fn titles(result: &SurfacingResult) -> Vec<&str> {
        // Collect just the titles so ordering assertions stay readable.
        result.items.iter().map(|i| i.title.as_str()).collect()
    }

    // ── Threshold: what surfaces on the day ──────────────────────────────────

    #[test]
    fn empty_input_surfaces_nothing() {
        // The designed empty state is a real result, not an error or a spinner:
        // with no candidates the Today view says "nothing today" and means it.
        // Rank an empty day.
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
    fn scheduled_instance_surfaces_on_its_day() {
        // A recurring instance whose scheduled day is today is the canonical thing
        // the Today view exists to show — the base case for the whole feature.
        // One open instance scheduled for this morning.
        let c = candidate("take out bins", TaskStatus::Open, sched(morning()));
        // Rank the single-candidate day.
        let result = rank(vec![c]);
        // Exactly one item comes back.
        assert_eq!(result.items.len(), 1);
        // The empty-state flag flips true because something surfaced.
        assert!(result.has_surfaced);
        // The title is preserved verbatim for display.
        assert_eq!(result.items[0].title, "take out bins");
        // A same-day instance is on time, not overdue.
        assert!(
            !result.items[0].overdue,
            "a same-day instance is not overdue"
        );
    }

    #[test]
    fn scheduled_instance_on_another_day_stays_off_today() {
        // Tomorrow's instance has a real scheduled time — but not today's — so it
        // must not clutter the current day.
        // One open instance scheduled for tomorrow morning.
        let c = candidate("water plants", TaskStatus::Open, sched(tomorrow()));
        // Rank today's day for it.
        let result = rank(vec![c]);
        // A different day means no claim on today.
        assert!(result.items.is_empty());
        // And nothing surfaced.
        assert!(!result.has_surfaced);
    }

    #[test]
    fn one_shot_due_today_surfaces() {
        // A one-shot task whose deadline falls later today is due now and belongs
        // on Today. The deadline (evening) is still after `now`, so it is not
        // overdue — an ordinary surfaced item.
        // One open task due this evening.
        let c = candidate("call dentist", TaskStatus::Open, win(None, Some(evening())));
        // Rank the single-candidate day.
        let result = rank(vec![c]);
        // It surfaces as one item.
        assert_eq!(result.items.len(), 1);
        // And it is not overdue, because the deadline is still ahead.
        assert!(!result.items[0].overdue);
    }

    #[test]
    fn overdue_open_task_surfaces_as_information() {
        // A deadline that has already passed, on a task still open, must not
        // silently disappear — an open loop is exactly what surfacing exists to
        // hold. It surfaces flagged overdue, which the frontend renders as plain
        // information ("due earlier"), never a red badge or a count of days late.
        // One open task whose deadline was yesterday.
        let c = candidate(
            "renew passport",
            TaskStatus::Open,
            win(None, Some(yesterday())),
        );
        // Rank today's day for it.
        let result = rank(vec![c]);
        // It must still be present.
        assert_eq!(
            result.items.len(),
            1,
            "an open overdue task must not vanish"
        );
        // And it carries the overdue flag.
        assert!(result.items[0].overdue, "it must be flagged as overdue");
    }

    #[test]
    fn overdue_but_resolved_task_stays_quiet() {
        // The same past deadline, but the task is Done. Resolved tasks are settled
        // history, not open loops, so Today stays quiet — the status filter runs
        // ahead of the temporal rule.
        // One Done task whose deadline was yesterday.
        let c = candidate(
            "renew passport",
            TaskStatus::Done,
            win(None, Some(yesterday())),
        );
        // Rank today's day for it.
        let result = rank(vec![c]);
        // Terminal statuses never surface, overdue or not.
        assert!(result.items.is_empty(), "resolved tasks never surface");
    }

    #[test]
    fn task_inside_its_window_surfaces() {
        // A one-shot actionable since yesterday and due tomorrow is workable today:
        // the day sits inside [earliest, due]. It surfaces, and is not overdue
        // because the deadline is still in the future.
        // Earliest was yesterday, due is two days out — today is mid-window.
        let earliest = datetime!(2026-07-23 00:00:00 UTC);
        let due = datetime!(2026-07-25 23:00:00 UTC);
        // One open task spanning that window.
        let c = candidate(
            "prep sinterklaas",
            TaskStatus::Open,
            win(Some(earliest), Some(due)),
        );
        // Rank today's day for it.
        let result = rank(vec![c]);
        // Present, because today is inside the window.
        assert_eq!(result.items.len(), 1);
        // Not overdue, because the deadline is still ahead.
        assert!(!result.items[0].overdue);
    }

    #[test]
    fn task_before_its_window_opens_stays_quiet() {
        // A task not actionable until next week has a window that opens after
        // today. Showing it now would be premature nagging, so it stays off Today.
        // Both bounds are next week.
        let earliest = datetime!(2026-07-28 00:00:00 UTC);
        let due = datetime!(2026-07-30 00:00:00 UTC);
        // One open task whose window has not opened yet.
        let c = candidate(
            "book summer camp",
            TaskStatus::Open,
            win(Some(earliest), Some(due)),
        );
        // Rank today's day for it.
        let result = rank(vec![c]);
        // The window opens after today, so nothing surfaces.
        assert!(result.items.is_empty());
    }

    #[test]
    fn undated_task_makes_no_claim_on_today() {
        // With no earliest, no due, and no schedule, a task has no temporal claim
        // on any particular day. Amity errs toward silence: it waits to be chosen
        // rather than surfacing itself onto every day forever.
        // One open task with a fully-open window.
        let c = candidate("sort the loft", TaskStatus::Open, win(None, None));
        // Rank today's day for it.
        let result = rank(vec![c]);
        // A someday task stays off Today until it is given a time.
        assert!(result.items.is_empty(), "an undated task must stay quiet");
    }

    // ── Ordering and filtering ───────────────────────────────────────────────

    #[test]
    fn items_are_ordered_by_time_then_priority() {
        // Ordering is by time proximity first: the earliest salient time leads.
        // An overdue item (yesterday's deadline) sorts ahead of today's morning
        // instance, which sorts ahead of the afternoon one. Priority does not
        // override time here — it only breaks exact ties (see the next test).
        // Yesterday's deadline: the earliest salient time of the three.
        let overdue = candidate(
            "overdue thing",
            TaskStatus::Open,
            win(None, Some(yesterday())),
        );
        // This morning's instance: middle.
        let morning_item = candidate("morning thing", TaskStatus::Open, sched(morning()));
        // This afternoon's instance: latest.
        let afternoon_item = candidate("afternoon thing", TaskStatus::Open, sched(afternoon()));
        // Pass them out of order to prove the sort — not the input order — decides.
        let result = rank(vec![afternoon_item, morning_item, overdue]);
        // Earliest salient time first, all the way down.
        assert_eq!(
            titles(&result),
            vec!["overdue thing", "morning thing", "afternoon thing"]
        );
    }

    #[test]
    fn priority_breaks_ties_at_the_same_time() {
        // When two items share the exact same salient instant, the higher priority
        // comes first. This is the only place priority affects order — it never
        // lets an important task jump ahead of an earlier one.
        // Both instances are scheduled at the same instant.
        let at = morning();
        // The low-priority one (rank 2).
        let mut low = candidate("low", TaskStatus::Open, sched(at));
        low.priority = Some(Priority::new(2).unwrap());
        // The high-priority one (rank 5).
        let mut high = candidate("high", TaskStatus::Open, sched(at));
        high.priority = Some(Priority::new(5).unwrap());
        // Rank the two-item day.
        let result = rank(vec![low, high]);
        // Higher priority wins the tie.
        assert_eq!(titles(&result), vec!["high", "low"]);
    }

    #[test]
    fn member_filter_keeps_only_that_members_items() {
        // The "my today" filter narrows the day to one member: only items whose
        // current assignee matches survive, and everyone else's work is excluded.
        // With real members this powers per-person views; the logic runs now
        // against the placeholder member and a second, distinct one.
        // My task (assigned to the placeholder member by default).
        let mine = candidate("my task", TaskStatus::Open, sched(morning()));
        // Their task, reassigned to a different member the filter must drop.
        let mut theirs = candidate("their task", TaskStatus::Open, sched(afternoon()));
        theirs.current_assignee_id = Some(other_member());
        // Filter the day down to the placeholder member: only this member's
        // items should survive. `member_filter` is the config's one live lever.
        let config = SurfacingConfig {
            member_filter: Some(member()),
        };
        // Rank with the filter applied.
        let result = rank_today(vec![mine, theirs], today(), now(), &config);
        // Only the placeholder member's task survives.
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].title, "my task");
    }
}
