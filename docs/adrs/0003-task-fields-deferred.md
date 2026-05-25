# ADR-0003 — Task fields deferred from Task 2

**Date:** 2026-05-25
**Status:** Accepted

---

## Context

The brief (§6) enumerates a full set of task fields including project
membership, effort levels, priority scores, notes, and flexible due-date
windows. Task 2 implements the core Task entity but defers several fields to
keep the scope manageable and the initial schema stable.

This ADR records which fields are deferred, why, and what is needed before
each can be added. It exists so that future contributors understand the
rationale and do not re-introduce the complexity prematurely.

---

## Deferred fields

### `project_id`

The brief (§6.4) assigns tasks to a Project. Task 2 includes a nullable
`project_id` column in the schema but always sets it to `NULL` because the
Project entity does not yet exist.

**Why deferred:** Adding a FK constraint to a table that does not yet exist
would require either a fake "uncategorised" project row (polluting the data
model) or a nullable FK with no enforcement (same as the current state). The
nullable-NULL approach is already the current state, so nothing needs to change
when the Project entity lands — the `project_id` column just gets populated.

**What is needed:** The Project entity (its own task, after Task 2). Once
`projects` has a primary key, the FK can be enforced and the task creation
endpoint can accept an optional `project_id` field.

### `effort` and `priority`

The brief (§6.3) describes effort (a 1–5 integer) and priority (a 0–100
score) as fields that the household uses to balance load. The `tasks` table
includes both columns but the task creation and update endpoints do not yet
expose them.

**Why deferred:** These fields affect the "balance" and "suggest next task"
affordances, neither of which is implemented in Task 2. Surfacing the fields
in the API without the affordances that make them useful would invite clients
to set values with no observable effect — a confusing contract.

**What is needed:** The load-balancing or suggestion feature that reads
`effort` and `priority`. Once a consumer exists, the fields should be exposed
in the create/update request body.

### `notes`

Free-form markdown notes on the task. The column exists in the schema but is
not included in the create or update request bodies.

**Why deferred:** Notes require a safe markdown rendering pipeline on the
frontend. Accepting raw markdown without a rendering contract risks XSS when
the frontend eventually displays it. Task 2 keeps the field hidden until the
frontend's markdown renderer is confirmed.

**What is needed:** A markdown sanitiser in the frontend and a decision on
whether notes are stored as raw markdown or pre-rendered HTML.

### `due_by` and `earliest_at`

Flexible due windows: `due_by` is the deadline and `earliest_at` is the
earliest date the task should be displayed in the upcoming list. Both columns
exist in the schema.

**Why deferred:** These fields interact with the upcoming-instances query in
non-trivial ways (a task with `earliest_at` in the future should not appear
before that date, even if its `scheduled_at` has passed). Getting the query
semantics right requires user research and a clear spec. Task 2 defers that
complexity.

**What is needed:** A spec for how `earliest_at` and `due_by` interact with
the upcoming query, with test cases. The storage columns already exist.

---

## Fields intentionally included in Task 2

For completeness, the fields that **are** exposed in the Task 2 API:

| Field | Create | Update | Notes |
|---|---|---|---|
| `title` | required | optional | validated non-empty |
| `status` | server-set | via `/complete`, `/skip` | lifecycle state |
| `owner_id` | server-set | not exposed | seeded from migration |
| `current_assignee_id` | not in create | via `/assignee` | one-tap reassign |
| `tags` | optional array | optional array | normalised + stored as JSON |
| `recurrence_rrule` | optional | not yet | both or neither |
| `recurrence_timezone` | optional | not yet | paired with rrule |
| `created_at` | server-set | immutable | RFC 3339 |
| `updated_at` | server-set | server-set | RFC 3339 |

---

## Consequences

- The `tasks` table schema is a superset of what the API currently surfaces.
  This is intentional — schema migrations are expensive; adding columns to an
  existing table is cheap.
- Future tasks that expose deferred fields need only: (a) add the field to the
  request/response types, (b) write through to the existing column, and (c)
  update any affected tests. No migration is required for fields already in
  the schema.
- The `project_id_is_always_none_in_task_2` unit test in `amity-core` is a
  deliberate reminder: when the Project entity lands, that test should be
  deleted and replaced by a real project-assignment test.
