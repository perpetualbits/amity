// task.rs — the Task domain type.
//
// Task is the most-used entity in Amity — it models everything from weekly
// chores to seasonal maintenance to homework. See brief §6.5 for the full
// data model, and section 8 for the chore/household-maintenance context.
//
// This module defines:
//   • `TaskStatus`   — enum: Open, Doing, Done, Skipped.
//   • `EffortLevel`  — newtype u8 constrained to 1..=5.
//   • `Priority`     — newtype u8 constrained to 1..=5.
//   • `Task`         — the full domain struct for this task (live fields only).
//   • `TaskBuilder`  — builder that validates invariants before construction.
//   • `TaskError`    — error variants for builder and parse failures.
//
// Fields LIVE in this task (from the spec):
//   id, title, notes, status, due_by, earliest_at, effort, priority, tags,
//   owner_id, assignee_ids, eligible_member_ids, current_assignee_id,
//   recurrence, created_at, updated_at.
//
// Fields DEFERRED (column exists in migration, always None/empty until their
// entity lands):
//   project_id — reserved for the Project entity.
//   requires_ack — depends on real member management.
//
// Fields NOT YET ADDED (their own separate tasks):
//   checklist — ChecklistItem entity.
//   attachments — file-storage subsystem.
//
// See docs/adrs/0003-task-fields-deferred.md for the full rationale.
//
// Design note: the system records facts (who did what, when) and presents
// them for humans to interpret. It does NOT compute fairness scores, suggest
// reassignments, or shade overdue items in a threatening colour. See the
// philosophy: "The system makes the facts visible. The humans negotiate."

// Serde derives for JSON API and database round-trips.
use serde::{Deserialize, Serialize};
// OffsetDateTime carries the UTC offset explicitly — no naive datetimes.
use time::OffsetDateTime;

// Typed IDs for this entity and related entities.
use crate::ids::{MemberId, ProjectId, TaskId};
// RecurrenceRule is optional on Task; the materialiser turns it into instances.
use crate::recurrence::RecurrenceRule;

// ─── TaskStatus ───────────────────────────────────────────────────────────────

/// Lifecycle state of a Task.
///
/// For recurring tasks, status reflects the **Task as a whole** — it stays
/// `Open` until the recurrence ends (COUNT or UNTIL reached), at which point
/// it transitions to `Done`. Per-instance completion state lives in
/// `CompletionLog`, not here (see brief §6.5 and ADR-0003).
///
/// For one-shot tasks, the status follows the natural lifecycle.
// derive ord so statuses can be compared/sorted if needed in surfacing queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The task is open and available to work on.
    Open,

    /// Someone is actively working on this task.
    ///
    /// `Doing` is optional — not every workflow uses it. The hub touch UI
    /// supports it for tasks that span multiple sessions.
    Doing,

    /// The task has been completed (or the full recurrence series has ended).
    Done,

    /// The task was explicitly skipped.
    ///
    /// A skipped recurring instance writes a `CompletionLog` entry with
    /// `skipped=true` — skipping is a kind of completion event (brief §8.2).
    /// No debt accumulates; the next instance arrives on its normal schedule.
    Skipped,
}

impl std::fmt::Display for TaskStatus {
    /// Produce the `snake_case` storage string.
    ///
    /// These strings are the database storage contract. Changing them requires
    /// a migration to backfill existing rows. Do not change without an ADR.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual match keeps this independent of serde so it can be used in
        // non-serde contexts (e.g. SQL bind parameters).
        let s = match self {
            Self::Open => "open",
            Self::Doing => "doing",
            Self::Done => "done",
            Self::Skipped => "skipped",
        };
        // Write the selected storage string.
        write!(f, "{s}")
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = TaskError;

    /// Parse the `snake_case` storage string back into the enum.
    ///
    /// # Errors
    ///
    /// Returns `TaskError::UnknownStatus` if the string is not a known variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm must mirror the Display impl above to ensure round-trip safety.
        match s {
            "open" => Ok(Self::Open),
            "doing" => Ok(Self::Doing),
            "done" => Ok(Self::Done),
            "skipped" => Ok(Self::Skipped),
            other => Err(TaskError::UnknownStatus(other.to_owned())),
        }
    }
}

// ─── EffortLevel ─────────────────────────────────────────────────────────────

/// Subjective effort estimate for a Task, on a scale of 1 (very low) to 5 (very high).
///
/// This is a soft signal used for surfacing (brief §6.5 `effort?`). It is not
/// used to compute scores, quotas, or compliance metrics. The system never
/// derives a "workload" from effort levels; humans use them as a hint when
/// choosing what to take on.
///
/// 1 = a few minutes; 5 = most of an afternoon.
// Copy so callers can pass EffortLevel without move issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EffortLevel(u8);

impl EffortLevel {
    /// Construct an `EffortLevel` from a raw value.
    ///
    /// # Errors
    ///
    /// Returns `TaskError::InvalidEffortLevel` if `value` is outside `1..=5`.
    pub fn new(value: u8) -> Result<Self, TaskError> {
        // The valid range is documented in the brief and enforced here at the
        // domain boundary so storage can safely use INTEGER NOT NULL.
        if (1..=5).contains(&value) {
            Ok(Self(value))
        } else {
            Err(TaskError::InvalidEffortLevel(value))
        }
    }

    /// Return the raw numeric value (1..=5).
    #[must_use]
    pub fn value(self) -> u8 {
        // Expose the inner value for storage and serialisation.
        self.0
    }
}

// ─── Priority ────────────────────────────────────────────────────────────────

/// Importance rank for a Task, on a scale of 1 (lowest) to 5 (highest).
///
/// Like `EffortLevel`, this is a soft signal for surfacing order and personal
/// focus. The system surfaces high-priority tasks more prominently in the
/// Today view; it does not use priority to generate pressure or notifications.
// Copy for the same reasons as EffortLevel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Priority(u8);

impl Priority {
    /// Construct a `Priority` from a raw value.
    ///
    /// # Errors
    ///
    /// Returns `TaskError::InvalidPriority` if `value` is outside `1..=5`.
    pub fn new(value: u8) -> Result<Self, TaskError> {
        // Same constraint as EffortLevel — 1..=5 maps to the brief's model.
        if (1..=5).contains(&value) {
            Ok(Self(value))
        } else {
            Err(TaskError::InvalidPriority(value))
        }
    }

    /// Return the raw numeric value (1..=5).
    #[must_use]
    pub fn value(self) -> u8 {
        // Expose the inner value for storage and serialisation.
        self.0
    }
}

// ─── Task ─────────────────────────────────────────────────────────────────────

/// A household task — the most-used entity in Amity.
///
/// A Task can be one-shot (no recurrence) or recurring (with a `RecurrenceRule`
/// that the materialiser turns into `task_instances` rows). Both types share
/// the same struct; the difference is whether `recurrence` is `Some`.
///
/// Construction goes through [`TaskBuilder`] to enforce the invariants listed
/// on that type. Direct field construction is `pub` to allow the storage layer
/// to reconstruct tasks from database rows — callers outside the storage layer
/// should use the builder.
///
/// See `docs/amity_brief.md` §6.5 for the conceptual model.
// Debug for test inspection; Clone for cheap copies in handler code; PartialEq
// for assertions; Serialize/Deserialize for the JSON API and database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Globally unique, time-ordered identifier. See `amity-core::ids`.
    pub id: TaskId,

    /// One-line summary. Must not be empty after trimming.
    ///
    /// Stored verbatim after the builder validates non-emptiness. The system
    /// never infers a title from other fields.
    pub title: String,

    /// Optional free-form notes. No structure required; plain prose or a
    /// bullet list is equally valid.
    pub notes: Option<String>,

    /// Current lifecycle state. See [`TaskStatus`] for the semantics.
    pub status: TaskStatus,

    /// Latest meaningful completion time (a window, not a deadline).
    ///
    /// `None` means "whenever is appropriate". `Some(t)` means the task
    /// should ideally be done by time `t`. The system surfaces this without
    /// escalating tone — it is information, not pressure (brief §3).
    pub due_by: Option<OffsetDateTime>,

    /// Earliest time the task is meaningful to work on.
    ///
    /// Useful for windowed tasks that should not appear in the Today view
    /// until a particular date. `None` means "available now".
    pub earliest_at: Option<OffsetDateTime>,

    /// Subjective effort estimate, 1 (very low) to 5 (very high). Optional.
    pub effort: Option<EffortLevel>,

    /// Importance rank, 1 (lowest) to 5 (highest). Optional.
    pub priority: Option<Priority>,

    /// Free-form tags. Trimmed and lowercased on insert for consistent matching.
    ///
    /// Tags drive default behaviours: `chore`/`household` tags trigger
    /// holiday-skip; `outdoor` adds a weather context hint; `kid` controls
    /// Kid Mode visibility (brief §8.3).
    pub tags: Vec<String>,

    /// The member who owns this task (responsible for it existing).
    ///
    /// Uses the placeholder member ID until the Member entity is implemented.
    pub owner_id: MemberId,

    /// Members actively doing the work. May be empty (unassigned).
    pub assignee_ids: Vec<MemberId>,

    /// Members who are eligible to take on this task. Empty means anyone.
    ///
    /// This is the "volunteer pool" — not a hard constraint but a signal
    /// for the one-tap assignee-change affordance (brief §6.5).
    pub eligible_member_ids: Vec<MemberId>,

    /// The default member shown as responsible in the Today view.
    ///
    /// Freely changeable with a single tap. Not an obligation — a default
    /// that the household can override without ceremony (brief §8.2).
    pub current_assignee_id: Option<MemberId>,

    /// Recurrence rule, if this is a recurring task.
    ///
    /// `None` for one-shot tasks. The materialiser generates `task_instances`
    /// rows from this rule. See `recurrence_materialiser.rs`.
    pub recurrence: Option<RecurrenceRule>,

    /// Stub link to a Project. Always `None` until the Project entity lands.
    ///
    /// The column exists in the database so Task rows remain valid after the
    /// Project entity is added without a re-migration. See ADR-0003.
    pub project_id: Option<ProjectId>,

    /// When this task was first created.
    pub created_at: OffsetDateTime,

    /// When this task was last modified.
    pub updated_at: OffsetDateTime,
}

// ─── TaskBuilder ─────────────────────────────────────────────────────────────

/// Builder for [`Task`].
///
/// Enforces the invariants that the `Task` struct requires:
/// - `title` must not be empty after trimming.
/// - `owner_id` must be set.
/// - `now` must be set (used for `created_at` and `updated_at`).
///
/// Optional fields default to sensible values:
/// - `status` defaults to `Open`.
/// - `tags` are trimmed and lowercased during the `tag()` call.
/// - `assignee_ids`, `eligible_member_ids` default to empty vecs.
/// - `recurrence`, `project_id`, and all Option fields default to `None`.
// Debug enables inspection of partial builder state in test failure messages.
#[derive(Debug)]
pub struct TaskBuilder {
    /// The one-line task title. Required; validated on `build()`.
    title: Option<String>,

    /// Optional notes.
    notes: Option<String>,

    /// Lifecycle status; defaults to Open for new tasks.
    status: TaskStatus,

    /// Latest time the task should be done by.
    due_by: Option<OffsetDateTime>,

    /// Earliest time the task is meaningful to start.
    earliest_at: Option<OffsetDateTime>,

    /// Subjective effort estimate.
    effort: Option<EffortLevel>,

    /// Importance rank.
    priority: Option<Priority>,

    /// Normalised (trimmed+lowercased) tags.
    tags: Vec<String>,

    /// Member who owns this task. Required.
    owner_id: Option<MemberId>,

    /// Members currently doing the work.
    assignee_ids: Vec<MemberId>,

    /// Members eligible to take this on.
    eligible_member_ids: Vec<MemberId>,

    /// The default displayed assignee.
    current_assignee_id: Option<MemberId>,

    /// Recurrence pattern, if recurring.
    recurrence: Option<RecurrenceRule>,

    /// Stub project link; always None in Task 2.
    project_id: Option<ProjectId>,

    /// Current wall-clock time used for `created_at` and `updated_at`.
    now: Option<OffsetDateTime>,
}

impl TaskBuilder {
    /// Start building a new task.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Required fields start as None; build() checks for their presence.
            title: None,
            // notes is optional; None is a valid final state.
            notes: None,
            // New tasks start Open; the lifecycle begins here, not elsewhere.
            status: TaskStatus::Open,
            // Optional window fields; None means "no constraint".
            due_by: None,
            earliest_at: None,
            // Optional sizing hints; None means "not estimated".
            effort: None,
            priority: None,
            // tags starts empty; callers add tags via .tag() or .tags().
            tags: Vec::new(),
            // owner_id is required; None triggers a MissingField error in build().
            owner_id: None,
            // assignee and eligible lists start empty.
            assignee_ids: Vec::new(),
            eligible_member_ids: Vec::new(),
            // No default assignee until explicitly set.
            current_assignee_id: None,
            // One-shot by default; callers set this for recurring tasks.
            recurrence: None,
            // project_id is always None in Task 2 — the Project entity doesn't exist yet.
            project_id: None,
            // now must be supplied by the caller for deterministic tests.
            now: None,
        }
    }

    /// Set the task title.
    ///
    /// The title is stored verbatim (no normalisation); `build()` validates
    /// that the trimmed form is non-empty, but the stored value is the original.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        // Store as-is; validation (non-empty trim) is deferred to build().
        self.title = Some(title.into());
        self
    }

    /// Set optional free-form notes.
    #[must_use]
    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        // Wrap in Some to signal that notes were provided.
        self.notes = Some(notes.into());
        self
    }

    /// Set the lifecycle status (defaults to `Open`).
    ///
    /// The status is normally left at its default (`Open`) when creating tasks.
    /// This setter is used by the storage layer when reconstructing a task that
    /// was already in a non-`Open` state, and by tests that need specific states.
    #[must_use]
    pub fn status(mut self, status: TaskStatus) -> Self {
        // Override the Open default; useful when creating tasks that start Doing.
        self.status = status;
        self
    }

    /// Set the latest time the task should be completed.
    ///
    /// `due_by` is a soft signal — the system surfaces it but does not escalate
    /// or block actions when it is missed. A missed `due_by` is information for
    /// the household to act on, not a system fault (brief §3).
    #[must_use]
    pub fn due_by(mut self, due_by: OffsetDateTime) -> Self {
        // Wrap in Some; absence (None) means "no deadline" in the domain.
        self.due_by = Some(due_by);
        self
    }

    /// Set the earliest time the task is meaningful to start.
    ///
    /// Tasks with `earliest_at` in the future are hidden from the Today view
    /// until that time arrives. Useful for tasks that should not clutter the
    /// dashboard until their window opens (brief §6.5).
    #[must_use]
    pub fn earliest_at(mut self, earliest_at: OffsetDateTime) -> Self {
        // Wrap in Some; None means "immediately available".
        self.earliest_at = Some(earliest_at);
        self
    }

    /// Set the effort estimate. Returns an error if `value` is outside 1..=5.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::InvalidEffortLevel`] if `value` is not in `1..=5`.
    pub fn effort(mut self, value: u8) -> Result<Self, TaskError> {
        // Validate through EffortLevel::new rather than repeating the bounds check.
        self.effort = Some(EffortLevel::new(value)?);
        Ok(self)
    }

    /// Set the priority rank. Returns an error if `value` is outside 1..=5.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::InvalidPriority`] if `value` is not in `1..=5`.
    pub fn priority(mut self, value: u8) -> Result<Self, TaskError> {
        // Validate through Priority::new.
        self.priority = Some(Priority::new(value)?);
        Ok(self)
    }

    /// Add a single tag to this task.
    ///
    /// Tags are normalised (trimmed and lowercased) so that `"Chore"` and
    /// `"chore"` are the same tag. Empty tags (after trimming) are silently
    /// ignored.
    #[must_use]
    pub fn tag(mut self, tag: impl AsRef<str>) -> Self {
        // Normalise: trim whitespace and lowercase for consistent matching.
        let normalised = tag.as_ref().trim().to_lowercase();
        // Silently drop empty tags; they carry no information.
        if !normalised.is_empty() {
            self.tags.push(normalised);
        }
        self
    }

    /// Replace the tags list entirely with the provided set.
    ///
    /// Each tag is normalised (trimmed + lowercased) and empty tags are dropped.
    #[must_use]
    pub fn tags(mut self, tags: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        // Replace the current tag list; normalise every incoming value.
        self.tags = tags
            .into_iter()
            .map(|t| t.as_ref().trim().to_lowercase())
            // Filter out empty strings that result from all-whitespace inputs.
            .filter(|t| !t.is_empty())
            .collect();
        self
    }

    /// Set the owner of this task.
    ///
    /// Required. `build()` returns `MissingField("owner_id")` if absent.
    /// The owner is the member responsible for the task existing — not necessarily
    /// the member who does the work. See `current_assignee_id` for the doer.
    #[must_use]
    pub fn owner_id(mut self, owner_id: MemberId) -> Self {
        // Wrap in Some; absence triggers MissingField in build().
        self.owner_id = Some(owner_id);
        self
    }

    /// Add a member to the assignee list (those actively doing the work).
    ///
    /// Multiple calls accumulate members — the list is additive. Use `build()`
    /// directly after if no assignees should be set (the list starts empty).
    #[must_use]
    pub fn assignee(mut self, member_id: MemberId) -> Self {
        // Push to the existing list; callers add one at a time.
        self.assignee_ids.push(member_id);
        self
    }

    /// Add a member to the eligible-member list (those who may take this on).
    ///
    /// An empty eligible list means "any household member may take this on".
    /// A non-empty list restricts the one-tap assignee affordance to listed members.
    #[must_use]
    pub fn eligible_member(mut self, member_id: MemberId) -> Self {
        // Push to the eligible list; each call adds one member.
        self.eligible_member_ids.push(member_id);
        self
    }

    /// Set the current default assignee (shown in the Today view).
    ///
    /// This is the member shown as responsible in surfacing views. It is freely
    /// changeable with a single tap and carries no obligation (brief §8.2).
    #[must_use]
    pub fn current_assignee(mut self, member_id: MemberId) -> Self {
        // Wrap in Some; None means "no default assignee currently shown".
        self.current_assignee_id = Some(member_id);
        self
    }

    /// Attach a recurrence rule (makes this a recurring task).
    ///
    /// Without a recurrence rule the task is one-shot: it completes once and
    /// no new instances are generated. With a rule, the materialiser produces
    /// `task_instances` rows up to the 60-day horizon (ADR-0002).
    #[must_use]
    pub fn recurrence(mut self, rule: RecurrenceRule) -> Self {
        // Wrap in Some; None means one-shot behaviour.
        self.recurrence = Some(rule);
        self
    }

    /// Supply the current wall-clock time for `created_at` and `updated_at`.
    ///
    /// Production callers pass `OffsetDateTime::now_utc()`; tests pass a
    /// fixed value for determinism. This avoids the global-clock anti-pattern.
    ///
    /// Required. `build()` returns `MissingField("now")` if absent.
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Validate all invariants and construct the `Task`.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::EmptyTitle`] if `title` is absent or all-whitespace.
    /// Returns [`TaskError::MissingField`] if `owner_id` or `now` were not set.
    pub fn build(self) -> Result<Task, TaskError> {
        // Validate title first — it is the most visible required field.
        let title = self
            .title
            .ok_or_else(|| TaskError::MissingField("title".to_owned()))?;

        // Reject blank titles. Trailing/leading whitespace is not a title —
        // an accidental tap or a bug would otherwise create invisible tasks.
        if title.trim().is_empty() {
            return Err(TaskError::EmptyTitle);
        }

        // Both owner_id and now are required for a complete Task.
        let owner_id = self
            .owner_id
            .ok_or_else(|| TaskError::MissingField("owner_id".to_owned()))?;

        let now = self
            .now
            .ok_or_else(|| TaskError::MissingField("now".to_owned()))?;

        // Generate a new time-ordered ID at construction time.
        let id = TaskId::new();

        // Both created_at and updated_at start equal to `now`; updates bump
        // only updated_at via the repository's UPDATE statement.
        Ok(Task {
            id,
            // title is stored verbatim — only validation (not normalisation) was applied.
            title,
            // notes, due_by, earliest_at etc. pass through unchanged.
            notes: self.notes,
            status: self.status,
            due_by: self.due_by,
            earliest_at: self.earliest_at,
            effort: self.effort,
            priority: self.priority,
            // tags were already normalised in .tag() / .tags().
            tags: self.tags,
            owner_id,
            // Vec fields default to empty vecs if no members were added.
            assignee_ids: self.assignee_ids,
            eligible_member_ids: self.eligible_member_ids,
            current_assignee_id: self.current_assignee_id,
            recurrence: self.recurrence,
            // project_id is always None in Task 2 — Project entity is deferred.
            project_id: self.project_id,
            // Both timestamps start at `now`; storage updates bump updated_at separately.
            created_at: now,
            updated_at: now,
        })
    }
}

impl Default for TaskBuilder {
    /// Returns a builder with no fields set and default list/status values.
    fn default() -> Self {
        // Delegate to new() — single source of truth for the initial state.
        Self::new()
    }
}

// ─── TaskError ────────────────────────────────────────────────────────────────

/// Errors produced when building or parsing `Task`-related types.
///
/// These error types are returned to callers who need to report validation
/// failures to the user or to the HTTP response body. The `#[error(...)]`
/// attributes produce human-readable messages suitable for API responses.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    /// `title` was present but contained only whitespace.
    ///
    /// The service layer maps this to HTTP 422 Unprocessable Entity.
    #[error("task title must not be empty")]
    EmptyTitle,

    /// A required builder field was not set.
    ///
    /// The enclosed string names the field so callers can construct a helpful
    /// error message (e.g. "required field not set: `owner_id`").
    #[error("required field not set: {0}")]
    MissingField(String),

    /// The database contained an unrecognised `status` string.
    ///
    /// This indicates either a data-migration gap or a bug in the storage layer.
    /// A newer binary may have written a `status` value the current binary does
    /// not know about; the enclosed string names the unknown value.
    #[error("unknown task status: {0}")]
    UnknownStatus(String),

    /// An `effort` value outside the 1..=5 range was provided.
    ///
    /// The enclosed `u8` is the bad value, included so the error message can
    /// tell the client exactly what was wrong (e.g. "effort level 0 is out of range").
    #[error("effort level {0} is out of range (must be 1..=5)")]
    InvalidEffortLevel(u8),

    /// A `priority` value outside the 1..=5 range was provided.
    ///
    /// Same pattern as `InvalidEffortLevel` — the enclosed value is the bad input.
    #[error("priority {0} is out of range (must be 1..=5)")]
    InvalidPriority(u8),
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Placeholder member ID (matches migration 0001 value).
    fn placeholder() -> MemberId {
        // Using the same UUID as the placeholder inserted by migration 0001.
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    /// Fixed "now" for deterministic tests.
    fn fixed_now() -> OffsetDateTime {
        // An arbitrary fixed timestamp; determinism matters more than the exact value.
        datetime!(2026-05-25 10:00:00 UTC)
    }

    /// Build a minimal valid task with just the required fields.
    fn minimal_task() -> Task {
        TaskBuilder::new()
            .title("take out bins")
            .owner_id(placeholder())
            .now(fixed_now())
            .build()
            .expect("minimal task must be valid")
    }

    #[test]
    fn build_minimal_task() {
        // Happy path: only the required fields are set; defaults apply everywhere else.
        let task = minimal_task();
        // Title must be preserved verbatim.
        assert_eq!(task.title, "take out bins");
        // New tasks default to Open — no other lifecycle state makes sense for a new capture.
        assert_eq!(task.status, TaskStatus::Open);
        // Optional fields must all be absent when not set.
        assert!(task.notes.is_none());
        assert!(task.due_by.is_none());
        assert!(task.recurrence.is_none());
        // Lists start empty.
        assert!(task.assignee_ids.is_empty());
        assert!(task.tags.is_empty());
        // Timestamps must both equal `now`.
        assert_eq!(task.created_at, fixed_now());
        assert_eq!(task.updated_at, fixed_now());
    }

    #[test]
    fn empty_title_is_rejected() {
        // A blank title is not a valid task; the builder must enforce this.
        //
        // The empty string check covers the case where the client sends a title
        // that is nothing but whitespace — an accidental tap or a client bug.
        // The server must reject these before they reach the database.
        let result = TaskBuilder::new()
            .title("   ")
            .owner_id(placeholder())
            .now(fixed_now())
            .build();
        assert!(
            matches!(result, Err(TaskError::EmptyTitle)),
            "expected EmptyTitle, got {result:?}"
        );
    }

    #[test]
    fn missing_owner_id_is_rejected() {
        // owner_id is required because the database has a NOT NULL FK constraint.
        //
        // Forgetting to call .owner_id() in the builder produces a clear error
        // rather than a cryptic DB constraint violation at insert time.
        let result = TaskBuilder::new()
            .title("check boiler")
            .now(fixed_now())
            // No .owner_id() — this is the case under test.
            .build();
        assert!(matches!(result, Err(TaskError::MissingField(_))));
    }

    #[test]
    fn missing_now_is_rejected() {
        // `now` is required so tests can be deterministic and the service layer
        // always controls the clock (no hidden calls to OffsetDateTime::now_utc()
        // inside the builder that would make tests time-dependent).
        let result = TaskBuilder::new()
            .title("check boiler")
            .owner_id(placeholder())
            // No .now() call — this is the case under test.
            .build();
        assert!(matches!(result, Err(TaskError::MissingField(_))));
    }

    #[test]
    fn tags_are_normalised() {
        // Tags must be trimmed and lowercased regardless of input case or whitespace.
        //
        // The normalisation here is what makes `chore` and `CHORE` match in
        // queries and in the holiday-skip filter. Enforcing it at construction time
        // means the database always contains clean lowercase tags; no query-time
        // LOWER() is required.
        let task = TaskBuilder::new()
            .title("garden tasks")
            .owner_id(placeholder())
            .now(fixed_now())
            // Mix of cases and leading/trailing spaces — should all normalise to lowercase.
            .tag("  CHORE  ")
            .tag("Outdoor")
            .tag("  ") // blank — must be silently ignored
            .build()
            .unwrap();
        // Should have exactly two non-empty normalised tags.
        assert_eq!(task.tags, vec!["chore", "outdoor"]);
    }

    #[test]
    fn effort_level_bounds_are_enforced() {
        // 0 is below the minimum; must fail.
        // The bounds test both the out-of-range rejection and the boundary values
        // (1 and 5 must succeed) to confirm the closed range semantics.
        let err = TaskBuilder::new()
            .effort(0)
            .expect_err("0 must be rejected");
        assert!(matches!(err, TaskError::InvalidEffortLevel(0)));

        // 6 is above the maximum; must fail.
        let err = TaskBuilder::new()
            .effort(6)
            .expect_err("6 must be rejected");
        assert!(matches!(err, TaskError::InvalidEffortLevel(6)));

        // 1 and 5 are on the boundary and must succeed.
        assert!(TaskBuilder::new().effort(1).is_ok());
        assert!(TaskBuilder::new().effort(5).is_ok());
    }

    #[test]
    fn priority_bounds_are_enforced() {
        // Same bounds as EffortLevel (1..=5). Both newtypes share the same valid range.
        assert!(TaskBuilder::new().priority(0).is_err());
        assert!(TaskBuilder::new().priority(6).is_err());
        assert!(TaskBuilder::new().priority(1).is_ok());
        assert!(TaskBuilder::new().priority(5).is_ok());
    }

    #[test]
    fn task_status_round_trips_through_string() {
        // All status variants must survive Display→FromStr without loss.
        //
        // This test guards the storage contract: if Display or FromStr diverge,
        // rows already in the database cannot be read back correctly. Any change
        // to these strings requires a migration to update existing rows.
        for status in [
            TaskStatus::Open,
            TaskStatus::Doing,
            TaskStatus::Done,
            TaskStatus::Skipped,
        ] {
            let s = status.to_string();
            let parsed: TaskStatus = s.parse().expect("known status string");
            assert_eq!(status, parsed, "round-trip failed for {status}");
        }
    }

    #[test]
    fn unknown_status_returns_error() {
        // Simulates a newer binary writing a status value an older binary cannot read.
        // The enclosed string carries the bad value for logging and error messages.
        let result: Result<TaskStatus, _> = "deleted".parse();
        assert!(matches!(result, Err(TaskError::UnknownStatus(_))));
    }

    #[test]
    fn recurring_task_carries_rule() {
        // A task with a recurrence rule must preserve it through the builder.
        // The rule is stored opaquely — the domain does not inspect it here;
        // the materialiser validates it when generating instances.
        let rule = RecurrenceRule::new("FREQ=WEEKLY;BYDAY=TH", "Europe/Amsterdam");
        let task = TaskBuilder::new()
            .title("take out bins")
            .owner_id(placeholder())
            .now(fixed_now())
            .recurrence(rule.clone())
            .build()
            .unwrap();
        // The recurrence must be the exact rule we set.
        assert_eq!(task.recurrence, Some(rule));
    }

    #[test]
    fn project_id_is_always_none_in_task_2() {
        // The Project entity is deferred; project_id is always None until it lands.
        let task = minimal_task();
        assert!(
            task.project_id.is_none(),
            "project_id must be None until the Project entity is implemented"
        );
    }
}
