// completion_log.rs — the CompletionLog domain type.
//
// CompletionLog is the immutable record of a task instance being completed
// or skipped. It is the substrate for the household's own fairness conversations
// — facts laid out for humans to interpret.
//
// CRITICAL DESIGN NOTE (from brief §2 and §8.1):
//   The system does NOT compute a score, propose a redistribution, or shade an
//   item red because someone has been falling behind. CompletionLog is facts.
//   Humans decide what those facts mean. Any feature that derives a "fairness
//   metric" from this data violates the categorical commitments.
//
// Skipping is a kind of completion event (brief §8.2). A skipped instance
// writes a CompletionLog row with `skipped=true`. There is no separate
// "skip log" — the uniform history makes it easy to display without
// special-casing.
//
// This module is pure domain logic — no I/O, no async, no database types.
// Persistence is handled by `amity-storage::completion_log`.

// Serde derives for JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// `Date` for the instance_date field (a calendar day, not a point in time).
// `OffsetDateTime` for the precise completion timestamp.
use time::{Date, OffsetDateTime};

// Typed IDs for this entity and the entities it references.
use crate::ids::{CompletionLogId, MemberId, TaskId};

// ─── CompletionLog ────────────────────────────────────────────────────────────

/// An immutable record of a Task instance being completed, skipped, or resolved.
///
/// Each time a household member marks a task done (or skips it), one
/// `CompletionLog` row is created. The row is never updated — it is an
/// append-only audit trail. Corrections are handled by adding a new row,
/// not by editing an existing one.
///
/// The `instance_date` field identifies WHICH scheduled occurrence was completed.
/// For one-shot (non-recurring) tasks, `instance_date` is either the task's
/// `due_by` date (if set) or today's date at completion time.
///
/// For recurring tasks, `instance_date` is the scheduled date of the instance
/// (the calendar date from the `task_instances` table).
///
/// See `docs/amity_brief.md` §6.5 (`CompletionLog`) and §8.2 (chore-completion
/// model) for the conceptual context.
///
/// # Design note
///
/// `CompletionLog` is deliberately read-only from the domain's perspective.
/// There is no builder; records are constructed directly with all required
/// fields. This reflects the fact that a completion event is atomic — it
/// either happened (all fields known) or it didn't (no record).
// Debug for test assertions; Clone for response construction; PartialEq for
// assertions; Serialize/Deserialize for JSON API and database round-trips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionLog {
    /// Unique identifier for this log entry.
    pub id: CompletionLogId,

    /// The task that was completed or skipped.
    pub task_id: TaskId,

    /// Which scheduled instance this log entry refers to.
    ///
    /// For recurring tasks: the scheduled date from `task_instances`.
    /// For one-shot tasks: the task's `due_by` date, or today if unset.
    pub instance_date: Date,

    /// The member who marked the task done or skipped.
    ///
    /// This is the member who performed the action — not necessarily the
    /// current assignee. Fairness is visible from this data; the system
    /// does not derive a score from it.
    pub completed_by: MemberId,

    /// When the action was taken. Includes timezone offset.
    pub completed_at: OffsetDateTime,

    /// Whether the instance was skipped rather than completed.
    ///
    /// `true` means the instance was explicitly skipped. A skipped instance
    /// writes to `CompletionLog` so the history is uniform and complete —
    /// "skipped" is a kind of completion event, not an absence of one.
    /// No debt accumulates; the next instance arrives on its normal schedule.
    pub skipped: bool,

    /// Optional free-form context provided by the member.
    ///
    /// E.g. "did it together with Anna" or "chimney sweep rescheduled to Nov".
    /// This field is informational only; the system never parses or acts on it.
    pub notes: Option<String>,
}

impl CompletionLog {
    /// Construct a completion log entry.
    ///
    /// All fields except `notes` are required. Use `skipped=true` for skip events.
    ///
    /// The `id` is generated automatically — callers do not supply it.
    #[must_use]
    pub fn new(
        task_id: TaskId,
        instance_date: Date,
        completed_by: MemberId,
        completed_at: OffsetDateTime,
        skipped: bool,
        notes: Option<String>,
    ) -> Self {
        Self {
            // Generate a fresh ID for this log entry at construction time.
            // Each completion event has its own unique ID; the UUID v7 format
            // embeds a timestamp so log entries are naturally time-ordered.
            id: CompletionLogId::new(),
            // task_id identifies which task was acted on.
            task_id,
            // instance_date identifies WHICH scheduled occurrence was resolved.
            instance_date,
            // completed_by records the actor — used for the household's history view.
            completed_by,
            // completed_at is the precise moment of the action, not the scheduled time.
            completed_at,
            // skipped=true for a deliberate skip, false for a genuine completion.
            skipped,
            // notes is None when the member provides no additional context.
            notes,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    /// Placeholder task ID for tests.
    fn some_task_id() -> TaskId {
        // Use a fixed UUID to avoid ordering dependencies between tests.
        TaskId(uuid::Uuid::parse_str("018f1a2b-0000-7000-8000-000000000010").unwrap())
    }

    /// Placeholder member ID (matches migration 0001 value).
    fn placeholder_member() -> MemberId {
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    #[test]
    fn completion_log_construction() {
        // Verify all fields are stored correctly in a standard completion entry.
        let task_id = some_task_id();
        let member = placeholder_member();
        let now = datetime!(2026-05-25 10:00:00 UTC);

        // A completed (not skipped) instance with no notes.
        let log = CompletionLog::new(
            task_id,
            date!(2026 - 05 - 28), // a Thursday — the scheduled instance date
            member,
            now,
            false, // completed, not skipped
            None,
        );

        // All fields must survive the construction.
        assert_eq!(log.task_id, task_id);
        assert_eq!(log.instance_date, date!(2026 - 05 - 28));
        assert_eq!(log.completed_by, member);
        assert_eq!(log.completed_at, now);
        assert!(!log.skipped, "skipped must be false for a completion event");
        assert!(log.notes.is_none(), "no notes were provided");
    }

    #[test]
    fn skip_event_sets_skipped_flag() {
        // Skipping a task instance writes a log entry with skipped=true.
        // There is no separate "skip log" — skipping is a completion event.
        let log = CompletionLog::new(
            some_task_id(),
            date!(2026 - 05 - 28),
            placeholder_member(),
            datetime!(2026-05-28 09:00:00 UTC),
            true, // this is a skip event
            Some("away this week".to_owned()),
        );

        // skipped must be true for a skip event.
        assert!(log.skipped, "skipped must be true for a skip event");
        // Notes are preserved when provided.
        assert_eq!(log.notes.as_deref(), Some("away this week"));
    }

    #[test]
    fn each_completion_log_has_unique_id() {
        // Two completion log entries for the same task must have distinct IDs.
        let log_a = CompletionLog::new(
            some_task_id(),
            date!(2026 - 05 - 21),
            placeholder_member(),
            datetime!(2026-05-21 10:00:00 UTC),
            false,
            None,
        );
        let log_b = CompletionLog::new(
            some_task_id(),
            date!(2026 - 05 - 28),
            placeholder_member(),
            datetime!(2026-05-28 10:00:00 UTC),
            false,
            None,
        );
        // Each entry must have a distinct ID.
        assert_ne!(log_a.id, log_b.id, "completion log IDs must be unique");
    }
}
