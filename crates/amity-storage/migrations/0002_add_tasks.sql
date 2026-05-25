-- Migration 0002 — Task entity, task instances, and completion log.
--
-- Adds three tables required for Task 2:
--   tasks          — the Task entity (brief §6.5)
--   task_instances — materialised recurrence instances (see ADR-0002)
--   completion_logs — CompletionLog immutable records (brief §6.5, §8.2)
--
-- Design notes (carried forward from migration 0001):
--   • UUIDs are stored as TEXT for human-readability and Postgres portability.
--   • Datetimes are ISO-8601 TEXT (e.g. "2026-06-01T08:00:00+02:00").
--   • STRICT mode on all tables enforces real type constraints.
--   • JSON arrays (tags, assignee_ids, eligible_member_ids) are stored as TEXT.
--     The Rust layer serialises/deserialises them; SQLite has no array type.
--
-- Live vs. reserved fields (see docs/adrs/0003-task-fields-deferred.md):
--   LIVE (populated by Task 2 code):
--     title, notes, status, due_by, earliest_at, effort, priority, tags,
--     owner_id, assignee_ids, eligible_member_ids, current_assignee_id,
--     recurrence_rrule, recurrence_timezone, created_at, updated_at.
--   RESERVED for future tasks (always NULL in Task 2, column present to avoid
--   a re-migration later):
--     project_id     — reserved for the Project entity.
--     requires_ack   — reserved for real member management.
--
-- Not yet added (their own separate tasks):
--   checklist, attachments — separate storage concerns.
--
-- Foreign key constraints reference `members(id)` with the placeholder member
-- from migration 0001. Real member management lands in a later task.

-- Enable FK enforcement for this migration session.
PRAGMA foreign_keys = ON;

-- ─── tasks ───────────────────────────────────────────────────────────────────
--
-- Corresponds to Task in amity-core::task and brief §6.5.
-- Columns prefixed with `-- RESERVED:` are present for schema stability but
-- are always NULL in Task 2.

CREATE TABLE IF NOT EXISTS tasks (
    -- Primary key: UUID v7, time-ordered. TEXT for portability (see migration 0001).
    id              TEXT     NOT NULL PRIMARY KEY,

    -- ── Live fields ────────────────────────────────────────────────────────
    -- One-line summary; non-empty validated in the domain layer before insert.
    title           TEXT     NOT NULL,

    -- Optional free-form notes; NULL when not provided.
    notes           TEXT,

    -- Lifecycle state; snake_case enum string (open|doing|done|skipped).
    -- Defaults to 'open' on insert; the Rust layer controls the value.
    status          TEXT     NOT NULL DEFAULT 'open',

    -- Latest meaningful completion time; NULL means "whenever is appropriate".
    -- ISO-8601 TEXT including timezone offset, e.g. "2026-06-01T08:00:00+02:00".
    due_by          TEXT,

    -- Earliest meaningful start; NULL means "available now".
    earliest_at     TEXT,

    -- Effort estimate 1-5; NULL when not estimated. INTEGER NOT NULL impossible
    -- because the field is optional — NULL explicitly means "not estimated".
    effort          INTEGER,

    -- Importance rank 1-5; NULL when not ranked.
    priority        INTEGER,

    -- Free-form tags as a JSON array string, e.g. '["chore","outdoor"]'.
    -- Always a valid JSON array; '[]' when no tags are set (never NULL).
    tags            TEXT     NOT NULL DEFAULT '[]',

    -- Member who owns this task. FK to members(id).
    owner_id        TEXT     NOT NULL REFERENCES members(id),

    -- Members actively doing the work. JSON array of UUID strings; '[]' if none.
    assignee_ids    TEXT     NOT NULL DEFAULT '[]',

    -- Members eligible to take this task. JSON array; '[]' means anyone.
    eligible_member_ids TEXT  NOT NULL DEFAULT '[]',

    -- The default displayed assignee. NULL when no default is set.
    current_assignee_id TEXT REFERENCES members(id),

    -- Recurrence rule (RRULE string, no "RRULE:" prefix). NULL for one-shot tasks.
    -- Split into two columns so each can be queried and indexed independently.
    recurrence_rrule      TEXT,
    -- IANA timezone for DST-correct materialisation; NULL iff recurrence_rrule NULL.
    recurrence_timezone   TEXT,

    -- ── Reserved fields (always NULL in Task 2) ───────────────────────────
    -- project_id: reserved for the Project entity (Task 3 or later).
    -- The column exists now so the Project migration is purely additive.
    project_id      TEXT,   -- RESERVED: always NULL until Project entity lands.

    -- requires_ack: reserved for per-member acknowledgement (depends on Member entity).
    requires_ack    TEXT,   -- RESERVED: always NULL until Member entity lands.

    -- ── Audit timestamps ──────────────────────────────────────────────────
    created_at      TEXT     NOT NULL,
    updated_at      TEXT     NOT NULL
) STRICT;

-- Index on status+due_by supports the "upcoming open tasks" surfacing query.
-- Most queries filter by status='open' and order by due_by ASC.
CREATE INDEX IF NOT EXISTS idx_tasks_status_due
    ON tasks (status, due_by ASC);

-- Index on owner_id for "all tasks owned by member X" queries.
CREATE INDEX IF NOT EXISTS idx_tasks_owner
    ON tasks (owner_id);

-- ─── task_instances ──────────────────────────────────────────────────────────
--
-- Materialised instances of recurring tasks. The background job extends this
-- table up to a 60-day horizon and prunes instances older than 30 days.
-- See docs/adrs/0002-recurrence-engine.md §materialisation-strategy.

CREATE TABLE IF NOT EXISTS task_instances (
    -- UUID v7 for each instance. Not derived from the parent task ID so that
    -- the CompletionLog can reference a specific instance row without ambiguity.
    id              TEXT     NOT NULL PRIMARY KEY,

    -- The parent recurring task.
    task_id         TEXT     NOT NULL REFERENCES tasks(id),

    -- When this instance is scheduled; ISO-8601 TEXT with timezone offset.
    -- The local-time components (hour, minute) are stable across DST transitions
    -- because the materialiser anchors to wall-clock time (brief §6.6).
    scheduled_at    TEXT     NOT NULL,

    -- The default assignee for this specific instance. Inherits from the
    -- most recent prior instance, or from the parent task if this is the first.
    -- Freely changeable; NULL until inherited or explicitly set.
    current_assignee_id TEXT REFERENCES members(id),

    -- Each task can have at most one instance at a given scheduled time.
    -- Duplicate materialisation (e.g. if the background job runs twice) is
    -- handled by INSERT OR IGNORE; the unique constraint prevents duplicates.
    UNIQUE (task_id, scheduled_at)
) STRICT;

-- Index on task_id + scheduled_at DESC supports the "next N instances for
-- task X" query used by the upcoming-tasks surfacing endpoint.
CREATE INDEX IF NOT EXISTS idx_task_instances_task_scheduled
    ON task_instances (task_id, scheduled_at ASC);

-- Index on scheduled_at supports the pruning query (DELETE WHERE scheduled_at < ?).
CREATE INDEX IF NOT EXISTS idx_task_instances_scheduled
    ON task_instances (scheduled_at ASC);

-- ─── completion_logs ─────────────────────────────────────────────────────────
--
-- Immutable records of task completion or skip events. Append-only.
-- See CompletionLog in amity-core::completion_log and brief §6.5, §8.2.
--
-- DESIGN NOTE: this table is the facts layer. The system does not compute
-- scores, fairness metrics, or recommendations from it. Humans read these
-- facts and decide what they mean (brief §2 categorical commitments).

CREATE TABLE IF NOT EXISTS completion_logs (
    -- UUID v7; each log entry has its own unique, time-ordered ID.
    id              TEXT     NOT NULL PRIMARY KEY,

    -- Which task was completed or skipped.
    task_id         TEXT     NOT NULL REFERENCES tasks(id),

    -- The scheduled date of the instance (YYYY-MM-DD).
    -- For recurring tasks: the date from task_instances.scheduled_at.
    -- For one-shot tasks: the task's due_by date, or today if unset.
    instance_date   TEXT     NOT NULL,

    -- Who performed the action (not necessarily the current assignee).
    completed_by    TEXT     NOT NULL REFERENCES members(id),

    -- Precise timestamp of the action, ISO-8601 with offset.
    completed_at    TEXT     NOT NULL,

    -- 1 = instance was skipped; 0 = instance was completed.
    -- Skipping is a completion event (no separate skip table) — brief §8.2.
    -- SQLite stores booleans as INTEGER 0/1 in STRICT mode.
    skipped         INTEGER  NOT NULL DEFAULT 0,

    -- Optional context from the member (e.g. "did it together with Anna").
    -- The system never parses this; it is informational only.
    notes           TEXT
) STRICT;

-- Index on task_id + completed_at DESC supports "history for task X" queries.
CREATE INDEX IF NOT EXISTS idx_completion_logs_task
    ON completion_logs (task_id, completed_at DESC);

-- Index on completed_by + completed_at DESC supports "history for member X" queries.
-- This is the view the household uses to see who has been doing what.
-- The system displays the data; humans interpret it (no computed score).
CREATE INDEX IF NOT EXISTS idx_completion_logs_member
    ON completion_logs (completed_by, completed_at DESC);
