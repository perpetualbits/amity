# Task 2 — Task entity, recurrence engine, and CompletionLog

*Claude Code task description. Read in full before starting; surface questions before writing code.*

---

## Context

Task 1 (repository scaffolding + Inbox entity end-to-end) landed cleanly. The Cargo workspace, the three-crate split (core, storage, service), the migration approach, the API surface conventions, and the Tauri shell are all in place and verified. The HTTP API was exercised end-to-end via curl; the Tauri frontend builds but wasn't run live due to a missing `libwebkit2gtk` dependency on the dev machine — that's an environmental issue, not a code issue.

Task 2 builds on that foundation by adding the **Task entity** — the most-used type in the system — along with the **recurrence engine** that will be reused for every other entity that supports recurring instances, and the **CompletionLog** that records who marked tasks done. Task 2 also folds in a small fix discovered during Task 1's demo.

Before writing any code, re-read:

- `docs/amity_philosophy.md` — the values; especially the section on "what we believe about human relationships" (no fairness arbitration, no auto-assignment, no compliance supervision).
- `docs/amity_brief.md` — particularly section 6.5 (Task entity), section 6.6 (recurrence/cadence), and section 8 (chores and home maintenance — the most important application of Task).
- `docs/rust_guidelines.md` — crate choices and patterns.
- `docs/coding_guidelines.md` — comment density requirement, ADR discipline.

## What Task 2 delivers

Three primary deliverables plus a punch-list item.

### 1. The `Task` entity

The full Task type per the brief's section 6.5, with a deliberate split between "in this task" and "deferred to a later task" fields. This is *not* the entire Task definition shipping at once.

**In this task:**

- `id: TaskId(Uuid)` — newtype, UUID v7.
- `title: String` — non-empty after trimming.
- `notes: Option<String>` — free-form.
- `status: TaskStatus` — enum: `Open`, `Doing`, `Done`, `Skipped`.
- `due_by: Option<OffsetDateTime>` — window, not point-in-time. Local-anchored.
- `earliest_at: Option<OffsetDateTime>` — earliest meaningful start, for windowed tasks.
- `effort: Option<EffortLevel>` — newtype around `u8` constrained to 1..=5. The `Effort` enum from the brief.
- `priority: Option<Priority>` — newtype, similar constraint.
- `tags: Vec<String>` — free-form, trimmed, lowercased on insert for matching.
- `owner_id: MemberId` — required for now; uses the placeholder member ID from Task 1 until member management lands.
- `assignee_ids: Vec<MemberId>` — those *doing* the work; may be empty.
- `eligible_member_ids: Vec<MemberId>` — who *may* take this on; empty means anyone.
- `current_assignee_id: Option<MemberId>` — the default; freely changeable.
- `recurrence: Option<RecurrenceRule>` — see section 2 below.
- `created_at`, `updated_at` — standard.

**Deferred to later tasks (the columns exist, set to NULL/empty):**

- `project_id: Option<ProjectId>` — Project entity doesn't exist yet; column added, always NULL until Project lands.
- `requires_ack: Option<RequiresAck>` — depends on real member management.
- `checklist: Vec<ChecklistItem>` — its own concern; comes in a follow-up task.
- `attachments: Vec<Attachment>` — touches storage subsystem; deserves its own task.

The migration adds the columns now (and stubs the types where necessary) so we don't have to re-migrate later. Comments in the migration make clear which fields are "live" vs. "reserved for future use".

### 2. Recurrence engine

The architecturally novel piece of Task 2. The Task entity's `recurrence: Option<RecurrenceRule>` field carries the rule. The engine handles parsing, validation, and instance materialisation.

**Implementation approach:**

- **DSL**: an **RRULE subset** per RFC 5545. Use the [`rrule`](https://crates.io/crates/rrule) crate (well-maintained, handles RFC 5545 properly).

- **Supported subset:**
  - `FREQ` — DAILY, WEEKLY, MONTHLY, YEARLY
  - `INTERVAL`
  - `BYDAY` — including positional prefixes (`1MO`, `-1FR` for "first Monday", "last Friday")
  - `BYMONTHDAY`
  - `UNTIL`
  - `COUNT`

- **Explicitly out of scope:** `BYWEEKNO`, `BYYEARDAY`, `BYHOUR`/`BYMINUTE`/`BYSECOND`, `BYSETPOS`, `WKST` overrides. The validation layer rejects rules using these features with a clear error. (A future task can extend support if needed.)

- **Materialisation strategy: hybrid.**
  - The rule is stored on the parent Task row.
  - Upcoming instances are materialised as separate `task_instances` rows in the database, generated up to a **60-day horizon** from the current date.
  - A background job (run on service start, plus daily) extends the horizon forward and prunes instances that have aged out beyond a small backward window (keep last 30 days for the CompletionLog references).

- **DST behaviour:** all recurring instances anchor to the household's local wall-clock time (Europe/Amsterdam by default). A weekly Thursday 08:00 task stays at 08:00 across the spring/autumn DST transitions. Tests verify this explicitly.

- **Holiday skip hook (implementation, not data):** the engine accepts an optional "holiday calendar" parameter — a set of dates to skip. For tasks tagged `chore` or `household`, the materialiser consults this set. *The holiday calendar source itself does not exist yet* — Task 2 introduces a stub that returns an empty set, with a clear `TODO` comment pointing to the future "load NL public holidays" task. The mechanism is in place; the data lands later.

- **currentAssigneeId on new instances:** when a new recurrence instance is materialised, its `current_assignee_id` inherits the value from the *most recent prior instance*. If there is no prior instance (first one being materialised), it inherits from the parent Task's `current_assignee_id`. No automatic rotation, no algorithm — just stable inheritance until a human changes it.

### 3. The `CompletionLog` entity

Records who marked a task done, when. The substrate for the household's visibility into who is doing what — without any computed score.

```rust
/// A record of a Task instance being completed, skipped, or otherwise resolved.
///
/// CompletionLog is the visible record of work done in the household. It is
/// the substrate for the household's own fairness conversations — facts laid
/// out for humans to interpret. The system does not compute a score or make
/// recommendations from this data; that would violate §C of the brief.
///
/// See `docs/amity_brief.md` section 8.2 for the chore-completion model.
pub struct CompletionLog {
    pub id: CompletionLogId,
    pub task_id: TaskId,             // the parent Task
    pub instance_date: Date,         // which scheduled instance (for recurring tasks)
    pub completed_by: MemberId,      // who actually marked it done
    pub completed_at: OffsetDateTime,
    pub skipped: bool,               // true if the instance was explicitly skipped
    pub notes: Option<String>,       // optional context ("did it together with Anna")
}
```

Marking a Task done writes a CompletionLog row. Marking it skipped writes a CompletionLog row with `skipped=true`. There is no separate "skip log" — skipping is just a kind of completion-event, deliberately, so the historical record is uniform.

### 4. Punch-list item: data directory auto-create

During Task 1's demo, the service crashed on first run because `~/.local/share/amity/` didn't exist. The fix: at service startup, before opening the database, ensure the data directory exists (create it with appropriate permissions if absent). Use the `directories` crate (already a clean dependency) for cross-platform path resolution.

This is one small commit early in Task 2, not a separate task.

## Deliverables (concrete)

### `amity-core`

- `Task` type in `crates/amity-core/src/task.rs`.
- `CompletionLog` type in `crates/amity-core/src/completion_log.rs`.
- Newtypes in `crates/amity-core/src/ids.rs`: `TaskId(Uuid)`, `CompletionLogId(Uuid)`, plus stub `ProjectId(Uuid)` (used only as Option<ProjectId> on Task; no Project entity yet).
- `RecurrenceRule` type and parser in `crates/amity-core/src/recurrence.rs`.
- `RecurrenceMaterialiser` in `crates/amity-core/src/recurrence_materialiser.rs` — takes a `RecurrenceRule`, a `start_from`, a `horizon`, and an optional `skip_dates: &HashSet<Date>`; returns a `Vec<OffsetDateTime>` of materialised instance times.
- Unit tests for: rule validation (rejecting unsupported features), DST behaviour across spring/autumn transitions, holiday-skip behaviour with a non-empty skip set, COUNT and UNTIL termination, positional BYDAY rules.

### `amity-storage`

- Migration `0002_add_tasks.sql` — creates `tasks` table, `task_instances` table, `completion_logs` table. Includes the foreign-key constraints to `members` (placeholder member still in use).
- Repository functions in `crates/amity-storage/src/task.rs`: `insert_task`, `fetch_task`, `list_tasks_with_filter`, `update_task`, `mark_task_done`, `mark_task_skipped`. The latter two write CompletionLog entries.
- Repository functions in `crates/amity-storage/src/task_instance.rs`: `insert_instance`, `fetch_upcoming_instances` (the surfacing query), `prune_old_instances`.
- Repository functions in `crates/amity-storage/src/completion_log.rs`: `insert_completion_log`, `list_completions_for_task`, `list_completions_for_member`.
- Integration tests covering the full insert-materialise-complete cycle.

### `amity-service`

- API endpoints in `crates/amity-service/src/api/task.rs`:
  - `POST /api/v1/tasks` — create a Task; if recurrence is set, materialise initial instances.
  - `GET /api/v1/tasks` — list tasks, with optional filters (status, tag, due-before).
  - `GET /api/v1/tasks/{id}` — fetch a single Task.
  - `PATCH /api/v1/tasks/{id}` — update a Task. If recurrence changes, re-materialise.
  - `POST /api/v1/tasks/{id}/complete` — mark done. Body: `{ "instance_date": "YYYY-MM-DD", "notes": "..." }`. Writes CompletionLog.
  - `POST /api/v1/tasks/{id}/skip` — mark skipped. Body: `{ "instance_date": "YYYY-MM-DD", "notes": "..." }`. Writes CompletionLog.
  - `POST /api/v1/tasks/{id}/assignee` — change `current_assignee_id`. The one-tap reassignment from the brief.
  - `GET /api/v1/tasks/upcoming` — the surfacing query: returns upcoming Task instances within a window. This is the start of the Today/Week view's data source.
- A background job in `crates/amity-service/src/jobs/recurrence_horizon.rs` that extends the materialisation horizon daily.
- Data directory auto-create at startup (the punch-list item).
- Integration tests in `crates/amity-service/tests/task_api.rs` covering the endpoints end-to-end.

### `apps/hub-tauri`

- Minimal Task capture UI: a form with title, optional notes, optional dueBy date picker, optional recurrence picker (initially just a small set of presets: "once", "daily", "weekly on [day]", "monthly on the [n]th"), tags input.
- A "Today's Tasks" list view showing upcoming Task instances within the next 24 hours.
- A "Mark done" button per item that calls `POST /api/v1/tasks/{id}/complete`.
- An "Reassign" affordance per item that opens a simple member picker. (With only the placeholder member existing, this is mostly UI scaffolding; it becomes real when member management lands.)
- All the same accessibility constraints as Task 1: Atkinson Hyperlegible, 60×60 minimum touch targets, plain calm visual style.

### Documentation

- ADR `docs/adrs/0002-recurrence-engine.md` — records the RRULE-subset decision, materialisation horizon choice, holiday-skip mechanism. Cross-reference the brief sections it implements.
- ADR `docs/adrs/0003-task-fields-deferred.md` — records which Task fields are live in Task 2 and which are reserved for future tasks. Short but important; prevents confusion later.
- Update `README.md` if needed to mention Task as the second entity available.

## Open Questions

These need maintainer input before commitment. Surface them at the planning stage.

### Recurrence picker UI in the Tauri frontend

The brief never described what a recurrence picker looks like. A small preset list ("once / daily / weekly on [day] / monthly on the [nth]") covers the common cases without exposing RRULE complexity to the user. A free-text "advanced" mode could accept raw RRULE for power users. For Task 2, I'd recommend preset-only; advanced RRULE input lands later. Confirm.

### Behaviour when a Task's recurrence is changed mid-flight

If a Task already has materialised instances and the user changes its recurrence rule, the existing future instances should be deleted and re-materialised from the new rule. Past instances (and their CompletionLog entries) are *preserved* untouched. Confirm this is the desired behaviour. The alternative — keeping already-materialised future instances but applying new rule going forward — is more complex and probably the wrong choice for a system where simplicity matters.

### "Done" semantics for non-recurring tasks

A non-recurring (one-shot) Task has no `instance_date` in the natural sense. The completion endpoint requires it. For one-shot tasks, the convention should be: use the Task's `due_by` date if set, otherwise use today's date. Confirm this convention or propose an alternative.

### When does a recurring Task's `status` change?

A recurring Task has many instances; some completed, some upcoming. What does the parent Task's `status` field mean in this context? Two options: (a) status applies to the *Task as a whole* and is only `Open` until the recurrence ends (UNTIL or COUNT reached), then becomes `Done`; (b) status applies to the *next upcoming instance* — but that breaks the model because per-instance status lives on the CompletionLog, not the Task. I'd recommend (a). Confirm.

## Acceptance criteria

Same baseline as Task 1, plus:

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo test --workspace` passes; specifically including the DST corner-case tests, the holiday-skip tests, the horizon-extension tests, and the assignee-inheritance tests.
- [ ] `cargo clippy --workspace --all-targets -- -W clippy::pedantic` passes.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo doc --workspace --no-deps --all-features` builds clean.
- [ ] Comment density audit passes for all new files (50% target).
- [ ] All commits DCO-signed and follow Conventional Commits.
- [ ] Both ADRs land and are substantive.
- [ ] Manual API exercise: create a recurring task with `FREQ=WEEKLY;BYDAY=TH`, verify instances materialise, mark one done, verify CompletionLog row, verify subsequent instances unaffected.
- [ ] Manual API exercise: create a task with an unsupported RRULE feature (e.g. `BYWEEKNO=10`), verify the API returns a clear 422 error pointing at the unsupported feature.
- [ ] Service auto-creates `~/.local/share/amity/` on startup if absent.

## Scope guardrails

This task does **not** include:

- Project entity. Only the `project_id` column on Task, always NULL.
- Real Member management. Placeholder member from Task 1 remains in use.
- Checklist items or attachments on Tasks. Columns/types stubbed but not implemented.
- The full Today view or Week view. The `GET /api/v1/tasks/upcoming` endpoint exposes the data; the rich rendering of it is a later task.
- Notification firing when a Task's `due_by` arrives. The data is there; the notification system is a separate concern.
- Event entity. Different task, different patterns to establish.
- The Inbox-to-Task triage flow ("turn this InboxItem into a Task"). Tempting but separate; comes after Task 2.

If work-in-progress is creeping outside these guardrails, stop and ask.

## Estimated effort

The Inbox entity in Task 1 was the foundation slice. Task is meaningfully larger because of the recurrence engine and CompletionLog. Estimate: **5–8 focused days**, vs. Task 1's 3–5. If the work is exceeding 8 days substantially, flag it — that's a signal that scope has expanded beyond what was specified here.

## Reading order suggestion

1. Re-read the philosophy section on human relationships (avoiding fairness arbitration).
2. Brief section 6.5 (Task), section 6.6 (recurrence), section 8 (chores).
3. The Rust guidelines on crate choices, especially around adding `rrule` as a new dependency (note that the rust-guidelines.md document doesn't list `rrule` as pre-approved; this is one new dependency that the spec is explicitly justifying).
4. This task description.
5. Plan, confirm with maintainer, implement.

---

*A note on the new `rrule` dependency: per the rust-guidelines and the working-agreements in `claude_code_workflow.md`, adding a dependency not on the pre-approved list normally requires maintainer confirmation. This spec is the maintainer confirmation for `rrule`. No further check needed.*
