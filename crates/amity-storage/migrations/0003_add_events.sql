-- Migration 0003 — Event entity, event instances, and event overrides.
--
-- Adds three tables for Task 4 (brief §6.5 Event, §7 calendars & time):
--   events           — the Event entity (native or read-only external)
--   event_instances  — materialised recurrence instances (mirrors task_instances)
--   event_overrides  — local overlays on read-only external instances
--
-- Design notes (carried forward from migrations 0001/0002):
--   • UUIDs are stored as TEXT for readability and Postgres portability.
--   • Datetimes are ISO-8601 / RFC 3339 TEXT with an explicit offset.
--   • Booleans are INTEGER 0/1 under STRICT mode.
--   • Recurrence is split into rrule + timezone columns, both NULL for one-shot
--     and external events (external recurrence is trusted to the source).
--   • The EventSource fields are flattened onto the events row with a
--     `source_` prefix rather than a separate table — a source is 1:1 with its
--     event and never queried independently.

-- Enable FK enforcement for this migration session.
PRAGMA foreign_keys = ON;

-- ─── events ──────────────────────────────────────────────────────────────────
--
-- Corresponds to Event in amity-core::event and brief §6.5.

CREATE TABLE IF NOT EXISTS events (
    -- Primary key: UUID v7, time-ordered. TEXT for portability.
    id                    TEXT     NOT NULL PRIMARY KEY,

    -- One-line title; non-empty validated in the domain layer before insert.
    title                 TEXT     NOT NULL,

    -- Start instant; RFC 3339 TEXT with offset. For all-day events this is
    -- local midnight in the event's timezone.
    start_at              TEXT     NOT NULL,

    -- End instant; NULL when the event has no meaningful end. When present it
    -- is at or after start_at (enforced in the domain layer).
    end_at                TEXT,

    -- 1 = all-day event (surfaces on its date without a clock time); 0 = timed.
    all_day               INTEGER  NOT NULL DEFAULT 0,

    -- IANA timezone the event is anchored to (e.g. "Europe/Amsterdam").
    timezone              TEXT     NOT NULL,

    -- Optional free-form location; NULL when not set.
    location              TEXT,

    -- Recurrence rule (RRULE string, no "RRULE:" prefix); NULL for one-shot and
    -- external events. Split into two columns like tasks (ADR-0002).
    recurrence_rrule      TEXT,
    -- IANA timezone for DST-correct materialisation; NULL iff recurrence_rrule NULL.
    recurrence_timezone   TEXT,

    -- ── EventSource (flattened) ────────────────────────────────────────────
    -- Origin: 'native' (editable) or 'ics' (read-only external).
    source_kind           TEXT     NOT NULL DEFAULT 'native',
    -- The event's id within its external source; NULL for native events.
    source_external_id    TEXT,
    -- Which external calendar it belongs to; NULL for native events.
    source_calendar_id    TEXT,
    -- 1 = read-only (external): edits are recorded as event_overrides, never
    -- written back. 0 = native (fully editable).
    source_read_only      INTEGER  NOT NULL DEFAULT 0,
    -- When last refreshed from the external source; NULL for native events.
    source_last_synced_at TEXT,

    -- ── Audit timestamps ────────────────────────────────────────────────────
    created_at            TEXT     NOT NULL,
    updated_at            TEXT     NOT NULL
) STRICT;

-- Index on start_at supports the surfacing query (events on a given day) and
-- ordering one-shot events by time.
CREATE INDEX IF NOT EXISTS idx_events_start
    ON events (start_at ASC);

-- ─── event_instances ─────────────────────────────────────────────────────────
--
-- Materialised instances of recurring events. Mirrors task_instances; events
-- have no assignee, so there is no current_assignee_id column.

CREATE TABLE IF NOT EXISTS event_instances (
    -- UUID v7 for each instance row.
    id            TEXT     NOT NULL PRIMARY KEY,

    -- The parent recurring event.
    event_id      TEXT     NOT NULL REFERENCES events(id),

    -- When this instance is scheduled; RFC 3339 TEXT with offset. Local-time
    -- components are stable across DST because the materialiser anchors to
    -- wall-clock time (brief §6.6).
    scheduled_at  TEXT     NOT NULL,

    -- At most one instance per event at a given scheduled time; makes
    -- re-materialisation idempotent via INSERT OR IGNORE.
    UNIQUE (event_id, scheduled_at)
) STRICT;

-- Index on (event_id, scheduled_at) for the "instances for event X" query.
CREATE INDEX IF NOT EXISTS idx_event_instances_event_scheduled
    ON event_instances (event_id, scheduled_at ASC);

-- Index on scheduled_at supports the pruning query.
CREATE INDEX IF NOT EXISTS idx_event_instances_scheduled
    ON event_instances (scheduled_at ASC);

-- ─── event_overrides ─────────────────────────────────────────────────────────
--
-- Local overlays on read-only external event instances. See EventOverride in
-- amity-core::event_override and brief §6.5.

CREATE TABLE IF NOT EXISTS event_overrides (
    -- UUID v7; each overlay has its own unique, time-ordered id.
    id                TEXT     NOT NULL PRIMARY KEY,

    -- The event whose instance is being overridden.
    source_event_id   TEXT     NOT NULL REFERENCES events(id),

    -- The calendar date of the instance this overlay applies to (YYYY-MM-DD).
    instance_date     TEXT     NOT NULL,

    -- What the overlay does: 'cancel' | 'reschedule' | 'annotate'.
    action            TEXT     NOT NULL,

    -- Action data: a new RFC 3339 time (reschedule), a note (annotate), or NULL.
    payload           TEXT,

    -- Who created the overlay.
    created_by        TEXT     NOT NULL REFERENCES members(id),

    -- When the overlay was created; RFC 3339 with offset.
    created_at        TEXT     NOT NULL
) STRICT;

-- Index on (source_event_id, instance_date) so surfacing can look up whether a
-- given instance has an override in one indexed probe.
CREATE INDEX IF NOT EXISTS idx_event_overrides_event_date
    ON event_overrides (source_event_id, instance_date);

-- Index on instance_date supports the "all overrides on day D" surfacing query.
CREATE INDEX IF NOT EXISTS idx_event_overrides_date
    ON event_overrides (instance_date);
