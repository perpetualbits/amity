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
// `Date` is the day we surface for; `OffsetDateTime` is the salient instant;
// `Duration` steps `Date` forward across the 7-day week window.
use time::{Date, Duration, OffsetDateTime};

// `MemberId` is the displayed assignee; `Priority` orders items within a day.
use crate::ids::MemberId;
use crate::task::Priority;

// ─── SurfacedKind ───────────────────────────────────────────────────────────

/// The entity type a surfaced item came from.
///
/// `Task`, `Event`, and `Meal` are live today; the remaining surfacable types
/// named in brief §6.4 — Project milestones, Thread prompts — join here later
/// without a change to the wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacedKind {
    /// A household Task (or a materialised instance of one).
    Task,
    /// A calendar Event (native, or a read-only external one).
    Event,
    /// A planned dinner Meal (P2 Slice 3) — surfaced on Today only, as a
    /// non-actionable informational item (no done/reassign affordance yet;
    /// that lands with the hub's meal-planning UI). Week does not surface
    /// meals — see `api/surfacing.rs::build_meal_candidates`'s doc comment.
    Meal,
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

    /// A household note from an `Annotate` override, if one applies to this
    /// instance; `None` for every candidate an override has not touched.
    pub annotation: Option<String>,

    /// True when a `Reschedule` override moved this instance's `timing` away
    /// from its originally-scheduled time. Tasks and un-overridden events
    /// always carry `false`.
    pub rescheduled: bool,
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

    /// A household note from an `Annotate` override on this instance, if any.
    pub annotation: Option<String>,

    /// True when a `Reschedule` override moved this instance to a new time.
    pub rescheduled: bool,
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
    // Project the surviving candidate into the wire item via the shared
    // mapping helper — `plan_week` calls the same one, so Today and Week never
    // drift on what a `SurfacedItem` carries.
    Some(candidate_to_item(candidate, at, overdue))
}

/// Project a surviving candidate into the wire `SurfacedItem`, given the
/// salient instant and overdue flag the caller's rule already resolved.
///
/// This is the ONE mapping from `SurfaceCandidate` to `SurfacedItem` in the
/// crate — `evaluate` (behind `rank_today`) and `plan_week` both call it, so a
/// field added to either type only ever has to be wired through once.
fn candidate_to_item(
    candidate: SurfaceCandidate,
    at: OffsetDateTime,
    overdue: bool,
) -> SurfacedItem {
    SurfacedItem {
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
        // Move the override-derived fields straight through — the rule does
        // not interpret them, it only carries what the caller resolved.
        annotation: candidate.annotation,
        rescheduled: candidate.rescheduled,
    }
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

// ─── Week planning ──────────────────────────────────────────────────────────
//
// `plan_week` is Week's counterpart to `rank_today`: same clock-injection
// discipline, same candidate shape, same shared `candidate_to_item` mapping —
// but it is a LAYOUT, not a ranking. There is no priority tiebreak and no
// "overdue, so it's sticky forever" behaviour; each candidate is placed on the
// one day its salient instant actually falls on, and the day either shows it
// or it does not. See the module-level design note in the module doc comment
// for how this differs from Today's rule.

/// One day's worth of surfaced items in a `WeekPlan`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeekDay {
    /// The calendar date this bucket is for.
    pub date: Date,
    /// The items placed on this day, already ordered (see `plan_week`).
    pub items: Vec<SurfacedItem>,
}

/// The outcome of laying out a 7-day window — the wire type behind
/// `GET /api/v1/week`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeekPlan {
    /// The Monday the week starts on.
    pub start: Date,
    /// Exactly 7 `WeekDay` buckets, `start` through `start + 6 days`, in order.
    pub days: Vec<WeekDay>,
}

/// Lay out the 7-day window starting at `week_start` (the Monday).
///
/// Unlike `rank_today`, this is not a ranked query — it is a placement of what
/// *is*: each live candidate is bucketed onto the single day its instance
/// falls on, in date order, with no priority scoring. The pipeline:
///
///   1. drop settled task candidates (done/skipped never appear; events are
///      always `Liveness::Live` already, so this only ever filters tasks —
///      see brief: "open (not-done) dated tasks only");
///   2. find the ONE day in the window each candidate's instance falls on, by
///      reusing `timing_on_day` per day and keeping the day whose date equals
///      the salient instant's own date (see the note on duplicate one-shot
///      candidates below);
///   3. apply `config.member_filter`, exactly as `rank_today` does;
///   4. map the survivor to a `SurfacedItem` via the shared `candidate_to_item`
///      helper, and push it into that day's bucket;
///   5. sort each bucket: all-day first, then events before tasks, then by
///      salient time ascending, then source id for a deterministic tiebreak.
///
/// Candidates outside `[week_start, week_start + 6]` — including a `Window`
/// task whose window never touches the week at all — are silently dropped, the
/// same "no claim, no surfacing" silence `rank_today` uses (module doc comment).
///
/// ## Why step 2 needs the `at.date() == day` guard, not a bare day loop
///
/// `timing_on_day`'s overdue-open branch (`due < now`) is deliberately
/// day-independent for Today: an overdue task should surface no matter which
/// day you ask about, because Today always means "right now". Naively
/// searching the 7 window days for the first one `timing_on_day` returns
/// `Some` for would hit that branch on EVERY day of the week and wrongly place
/// the task on `week_start` regardless of its real due date. Re-checking that
/// the returned salient instant's OWN date equals the day under test collapses
/// this back to exactly one placement — the due (or earliest) day — which is
/// the correct one for a layout that shows each thing once, where it belongs.
///
/// ## Why the caller may hand in duplicate one-shot task candidates
///
/// The service builds one day's task/event candidates at a time (reusing the
/// same per-day builders `GET /api/v1/surfacing/today` uses) and concatenates
/// all 7 days' candidates before calling this function. A recurring task or
/// event is naturally day-scoped by that builder (each day contributes only
/// its own instance), but a ONE-SHOT task's `Window` candidate is NOT — the
/// builder does not know or care which day it is being asked for, since the
/// rule alone decides relevance. Concatenated across 7 calls, the exact same
/// one-shot task therefore arrives as 7 *identical* candidates. Left alone
/// they would all resolve to the same placement day and appear 7 times over.
/// This function guards against that by skipping a candidate whose resulting
/// `(kind, source_id, at)` already has an entry in that day's bucket — the
/// only way three fields coincide exactly is that they are the same instance
/// arriving twice, since a genuinely distinct instance of a recurring series
/// always carries a distinct `at`.
#[must_use]
pub fn plan_week(
    candidates: Vec<SurfaceCandidate>,
    week_start: Date,
    now: OffsetDateTime,
    config: &SurfacingConfig,
) -> WeekPlan {
    // The 7 calendar dates this plan covers, Monday-first (whatever weekday
    // `week_start` actually is — the caller is trusted to hand in the Monday;
    // this function only lays out 7 consecutive days from it).
    let week_dates: Vec<Date> = (0_i64..7)
        .map(|offset| week_start + Duration::days(offset))
        .collect();

    // One accumulator bucket per day, indexed the same as `week_dates`.
    let mut buckets: Vec<Vec<SurfacedItem>> = vec![Vec::new(); 7];

    for candidate in candidates {
        // Settled tasks never appear (open-only rule); events are always Live
        // already, so this filter only ever removes tasks in practice.
        if candidate.liveness != Liveness::Live {
            continue;
        }

        // Find the single week day this candidate's instance lands on, if
        // any — see the doc comment above for why the extra date check matters.
        let placement = week_dates.iter().enumerate().find_map(|(idx, &day)| {
            let (at, overdue) = timing_on_day(candidate.timing, day, now)?;
            (at.date() == day).then_some((idx, at, overdue))
        });
        let Some((idx, at, overdue)) = placement else {
            // No day in the window claims this candidate — drop it silently.
            continue;
        };

        // Project into the wire item via the SAME mapping `rank_today` uses.
        let item = candidate_to_item(candidate, at, overdue);

        // "My week": narrow to one member's items, exactly as Today does.
        if !member_matches(&item, config) {
            continue;
        }

        // Collapse an exact-duplicate placement (same kind/source/instant) —
        // see the doc comment's note on repeated one-shot task candidates.
        let bucket = &mut buckets[idx];
        let already_placed = bucket.iter().any(|existing| {
            existing.kind == item.kind
                && existing.source_id == item.source_id
                && existing.at == item.at
        });
        if !already_placed {
            bucket.push(item);
        }
    }

    // Order each day: all-day first, then events before tasks, then by
    // salient time, then source id — a layout, not a ranking (no priority key).
    for bucket in &mut buckets {
        bucket.sort_by(|a, b| {
            b.all_day
                .cmp(&a.all_day)
                .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
                .then(a.at.cmp(&b.at))
                .then_with(|| a.source_id.cmp(&b.source_id))
        });
    }

    WeekPlan {
        start: week_start,
        days: week_dates
            .into_iter()
            .zip(buckets)
            .map(|(date, items)| WeekDay { date, items })
            .collect(),
    }
}

/// Ordering rank for the "events before tasks" layout rule — lower sorts first.
///
/// Only used by `plan_week`'s bucket sort; `rank_today` interleaves the two
/// kinds purely by time instead (ordering there is driven entirely by the
/// `all_day` flag, salient time, and priority — see `rank_today`'s sort), so
/// this has no equivalent there. `Meal` is ranked here only for the match to
/// stay exhaustive: `build_meal_candidates` (api/surfacing.rs) is wired into
/// Today alone, so a Meal candidate never actually reaches `plan_week` today.
/// It is placed alongside `Event` (both informational, non-actionable
/// context) so that if Week ever does grow a meal candidate, it leads tasks
/// by default without a further decision being needed here.
fn kind_rank(kind: SurfacedKind) -> u8 {
    match kind {
        // Events (and, if ever surfaced here, Meals) lead a day's timed
        // items, ahead of any task.
        SurfacedKind::Event | SurfacedKind::Meal => 0,
        SurfacedKind::Task => 1,
    }
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
            // No override machinery under test here — tasks never carry one.
            annotation: None,
            rescheduled: false,
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
            // Override fields are exercised by the service-layer e2e tests,
            // not the pure-rule tests here — default them off.
            annotation: None,
            rescheduled: false,
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

    // ── Week planning ─────────────────────────────────────────────────────────
    // Reuses the same fixed-day fixtures as the Today suite above: `today()`
    // (2026-07-24) is a Friday, so it falls at index 4 (Monday = 0) of the
    // week `week_start()` anchors — every timed fixture (`morning`,
    // `afternoon`, `evening`, `yesterday`) lands inside this same window.

    /// The Monday of the week `today()` falls in — `plan_week`'s fixed anchor.
    fn week_start() -> Date {
        // 2026-07-20 is a Monday; `today()` (2026-07-24, Friday) is 4 days in.
        date!(2026 - 07 - 20)
    }

    /// Plan the fixed week against the fixed `now`, with default config.
    fn plan(candidates: Vec<SurfaceCandidate>) -> WeekPlan {
        // Every test funnels through this call so the window and now stay fixed.
        plan_week(candidates, week_start(), now(), &SurfacingConfig::default())
    }

    /// Item counts per day, Monday first — a compact shape for asserting
    /// "only this one day got anything" without listing all 7 buckets by hand.
    fn day_counts(plan: &WeekPlan) -> Vec<usize> {
        plan.days.iter().map(|d| d.items.len()).collect()
    }

    #[test]
    fn produces_seven_monday_first_days() {
        // An empty week is still a real 7-day shape, not an empty vec — the
        // grid always has 7 columns even on a quiet week.
        let plan = plan(vec![]);
        assert_eq!(plan.start, week_start());
        assert_eq!(plan.days.len(), 7);
        // Each day is exactly one more than the last, starting at the Monday.
        let expected: Vec<Date> = (0_i64..7)
            .map(|offset| week_start() + Duration::days(offset))
            .collect();
        let actual: Vec<Date> = plan.days.iter().map(|d| d.date).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn bucketing_places_an_event_on_its_own_day_only() {
        // A Friday-morning event must land in the Friday bucket (index 4) and
        // nowhere else in the week.
        let e = event("dentist", sched(morning()), false);
        let plan = plan(vec![e]);
        assert_eq!(plan.days[4].date, today());
        assert_eq!(day_counts(&plan), vec![0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(plan.days[4].items[0].title, "dentist");
    }

    #[test]
    fn monday_start_boundary_keeps_the_edges_and_drops_the_neighbours() {
        // One day either side of the window (prior Sunday, next Monday) makes
        // no claim on this week; the window's own first and last day do.
        let before = event(
            "too early",
            sched(datetime!(2026-07-19 10:00:00 UTC)),
            false,
        );
        let after = event("too late", sched(datetime!(2026-07-27 10:00:00 UTC)), false);
        let first_day = event(
            "start of week",
            sched(datetime!(2026-07-20 06:00:00 UTC)),
            false,
        );
        let last_day = event(
            "end of week",
            sched(datetime!(2026-07-26 23:00:00 UTC)),
            false,
        );
        let plan = plan(vec![before, after, first_day, last_day]);
        assert_eq!(day_counts(&plan), vec![1, 0, 0, 0, 0, 0, 1]);
        assert_eq!(plan.days[0].items[0].title, "start of week");
        assert_eq!(plan.days[6].items[0].title, "end of week");
    }

    #[test]
    fn window_task_with_no_overlap_in_the_week_is_dropped() {
        // A one-shot task due well outside the window makes no claim on any
        // of its 7 days — same "no claim, no surfacing" silence as Today.
        let far_future = task(
            "someday",
            Liveness::Live,
            win(None, Some(datetime!(2026-08-15 09:00:00 UTC))),
        );
        let plan = plan(vec![far_future]);
        assert_eq!(day_counts(&plan), vec![0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn overdue_task_places_once_on_its_due_day_not_every_day() {
        // `yesterday()` (Thursday, index 3) is overdue relative to `now()`
        // (Friday noon). `timing_on_day`'s overdue branch is deliberately
        // day-independent for Today's "always sticky" behaviour — this proves
        // `plan_week` still collapses it to the ONE day it is actually due,
        // rather than placing it on all 7 days of the window.
        let overdue = task(
            "overdue thing",
            Liveness::Live,
            win(None, Some(yesterday())),
        );
        let plan = plan(vec![overdue]);
        assert_eq!(day_counts(&plan), vec![0, 0, 0, 1, 0, 0, 0]);
        assert!(plan.days[3].items[0].overdue);
    }

    #[test]
    fn all_day_leads_ahead_of_timed_events_and_tasks() {
        // All-day is the top sort key, ahead of both an earlier-clock-time
        // event and a task, mirroring Today's "banner" rule.
        let banner = event(
            "king's day",
            sched(datetime!(2026-07-24 00:00:00 UTC)),
            true,
        );
        let timed = event("school run", sched(morning()), false);
        let due_task = task("water plants", Liveness::Live, win(None, Some(evening())));
        let plan = plan(vec![timed, due_task, banner]);
        let items = &plan.days[4].items;
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "king's day");
    }

    #[test]
    fn events_lead_tasks_regardless_of_salient_time() {
        // Week's ordering rule is "events before tasks" as a hard group, not a
        // pure by-time interleave — unlike Today, which mixes the two kinds
        // purely by time. An earlier-due task still sorts after a later event.
        let early_task = task("early task", Liveness::Live, win(None, Some(morning())));
        let later_event = event("later event", sched(evening()), false);
        let plan = plan(vec![early_task, later_event]);
        let items = &plan.days[4].items;
        assert_eq!(
            items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            vec!["later event", "early task"]
        );
    }

    #[test]
    fn done_task_is_excluded_open_tasks_only() {
        // The maintainer's ratified default: Week shows only open tasks.
        let done = task("finished", Liveness::Settled, sched(morning()));
        let plan = plan(vec![done]);
        assert_eq!(day_counts(&plan), vec![0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn member_filter_narrows_the_week() {
        // Same "my week" narrowing as Today's per-person filter.
        let mine = task("mine", Liveness::Live, sched(morning()));
        let mut theirs = task("theirs", Liveness::Live, sched(afternoon()));
        theirs.current_assignee_id = Some(other_member());
        let config = SurfacingConfig {
            member_filter: Some(member()),
        };
        let plan = plan_week(vec![mine, theirs], week_start(), now(), &config);
        let items = &plan.days[4].items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "mine");
    }

    #[test]
    fn duplicate_one_shot_task_candidates_collapse_to_one_placement() {
        // Simulates the service concatenating 7 identical per-day builds of
        // the same one-shot task's `Window` candidate (see `plan_week`'s doc
        // comment) — without the dedupe guard this would place the task 7
        // times over on its due day.
        let original = task("water plants", Liveness::Live, win(None, Some(evening())));
        let duplicate = original.clone();
        let plan = plan(vec![original, duplicate]);
        assert_eq!(day_counts(&plan), vec![0, 0, 0, 0, 1, 0, 0]);
    }
}
