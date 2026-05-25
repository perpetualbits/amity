# ADR-0002 — Recurrence engine design

**Date:** 2026-05-25
**Status:** Accepted

---

## Context

Task 2 introduces recurring tasks — tasks that repeat on a schedule defined
by an iCalendar RRULE string (e.g. `FREQ=WEEKLY;BYDAY=TH`). The system must
materialise concrete "instance" rows from the abstract rule so that the
upcoming-tasks view can display them without re-evaluating the rule on every
request.

Several design questions must be settled:

1. **What format for the schedule rule?** The rule needs to be serialisable,
   human-readable, and processable by an existing library.
2. **Where do instances live?** Should they be computed on the fly or
   pre-materialised into database rows?
3. **How far ahead should the system materialise?** A fixed horizon or
   on-demand expansion?
4. **How are rule changes handled mid-flight?** If a household edits the
   RRULE after instances already exist, what happens to the existing rows?
5. **How are timezones handled?** DST-correct materialisation is required
   because household chores happen at local wall-clock times.

---

## Decision

### Rule format: iCalendar RRULE (subset)

RRULE is the established standard for repeating calendar events. The `rrule`
crate parses and iterates RRULE strings. Task 2 accepts a strict subset:

- **Allowed frequencies:** `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`.
- **Allowed modifiers:** `BYDAY`, `BYMONTHDAY`, `BYHOUR`, `UNTIL`, `COUNT`,
  `INTERVAL`, `WKST=MO`.
- **Rejected modifiers:** `SECONDLY`, `MINUTELY`, `HOURLY`, `BYYEARDAY`,
  `BYWEEKNO`, `BYSETPOS`, `BYSECOND`, `BYMINUTE`, `WKST` values other than
  `MO`.

This subset covers every household chore scheduling pattern encountered in
user research while excluding pathological cases (secondly/minutely) that
would produce millions of instances per year.

Validation happens at task creation time via `RecurrenceRule::from_str`. Tasks
with invalid or unsupported RRULE strings are rejected with a 422 before the
row is inserted.

The RRULE and its companion `recurrence_timezone` (IANA timezone identifier)
are stored together. Neither is valid without the other — the handler enforces
that both are supplied or neither is.

### Pre-materialisation into `task_instances` rows

Instances are materialised eagerly into the `task_instances` table rather than
computed on every request. The row contains:

- `id`: UUID v7
- `task_id`: FK to `tasks.id`
- `scheduled_at`: RFC 3339 UTC timestamp
- `current_assignee_id`: nullable FK to `members.id`

Materialisation happens synchronously in the POST `/tasks` handler immediately
after the task row is inserted. The handler calls `materialise_and_store_instances`
before returning 201, so instances are available for `GET /tasks/upcoming` in
the same request-response cycle.

**Why pre-materialisation rather than on-the-fly computation?**

- `GET /tasks/upcoming` is a hot path (shown on the hub's home screen). A
  simple indexed range query on `task_instances` is O(log n) per fetch;
  re-evaluating RRULE for every task on every request would be O(tasks ×
  instances) with no caching.
- Pre-materialisation decouples the recurrence engine (write path) from the
  query (read path), making both easier to test independently.
- The `INSERT OR IGNORE` upsert strategy means materialisation is idempotent:
  running it twice produces no duplicates, which makes restarts and concurrent
  runs safe.

### 60-day materialisation horizon

Instances are materialised for a rolling 60-day window ahead of the current
date. This is a hard-coded constant in `RecurrenceMaterialiser`.

**Why 60 days?**

- Long enough to show meaningful planning context (household can see what is
  coming up over the next two months).
- Short enough that a daily-recurring task produces at most 60 rows per task
  rather than thousands.
- Far enough ahead that the materialisation step does not need to run
  frequently — a daily cron job or a re-materialise-on-fetch strategy both
  work within the 60-day budget.

The 60-day window is not yet enforced by a background job in Task 2 (that
arrives in a later task). For now the window is materialised once at task
creation time.

### Mid-flight recurrence changes

When a recurring task's RRULE is changed after instances have already been
materialised, the system uses a "delete future, re-materialise" strategy:

1. `delete_future_instances(pool, task_id, now)` removes all instance rows
   with `scheduled_at >= now`.
2. `materialise_and_store_instances` re-runs with the new rule to fill the
   60-day window.

This keeps past instances (which may already be associated with completion
logs) intact. The `scheduled_at >= now` boundary is inclusive: the current
moment's instance is deleted and re-materialised under the new rule, preventing
a stale instance from lingering at the boundary.

**Why not update the existing rows?** The RRULE may produce a completely
different set of dates (e.g. changing from BYDAY=TH to BYDAY=MO). Updating
existing rows in-place would require a diff of old vs. new dates, which is
more complex than a delete + re-insert. The `INSERT OR IGNORE` upsert already
handles the idempotency concern.

### DST-correct materialisation via IANA timezones

Each recurring task carries a `recurrence_timezone` (e.g. `Europe/Amsterdam`).
Materialisation proceeds as follows:

1. Parse the RRULE `DTSTART` in the named timezone.
2. Iterate the rule using the `rrule` crate, which applies DST transitions.
3. Store the resulting `OffsetDateTime` values as UTC timestamps in `scheduled_at`.

This ensures that a task scheduled for "every Thursday at 08:00 Amsterdam time"
fires at 07:00 UTC in winter and 06:00 UTC in summer — the correct local time
in both cases.

The `rrule` crate handles IANA timezone expansion internally via its bundled
timezone database. No external tz-data lookup is required at runtime.

---

## Consequences

- The `task_instances` table grows by up to 60 rows per recurring task at
  creation time. For a household with 50 recurring tasks this is 3 000 rows —
  well within SQLite's practical limits.
- Adding a background job to refresh the materialisation window in a later
  task requires only calling `upsert_task_instances` on a schedule — the
  idempotency property means no duplicate-avoidance logic is needed at the
  call site.
- The 60-day constant is intentionally not configurable in Task 2. If household
  research later shows a different horizon is needed, it is a single-constant
  change in `amity-core/src/recurrence_materialiser.rs`.
