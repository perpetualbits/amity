-- 0004_add_calendars.sql — external calendar subscriptions (Task 5).
--
-- A `calendars` row is one subscribed read-only ICS feed. Its events live in the
-- existing `events` table (migration 0003), linked by `source_calendar_id` and
-- identified within the feed by `source_external_id` (the VEVENT UID). A partial
-- UNIQUE index on that pair makes re-sync an idempotent upsert; it is partial so
-- native events (NULL source_calendar_id) are never constrained.

CREATE TABLE calendars (
    id              TEXT    NOT NULL PRIMARY KEY,   -- UUID v7
    name            TEXT    NOT NULL,               -- display name (non-empty)
    url             TEXT    NOT NULL,               -- http(s) feed URL
    category        TEXT    NOT NULL,               -- snake_case CalendarCategory
    enabled         INTEGER NOT NULL,               -- 0/1
    created_at      TEXT    NOT NULL,               -- RFC 3339
    last_synced_at  TEXT,                           -- RFC 3339, NULL until first success
    last_status     TEXT    NOT NULL,               -- snake_case SyncStatus
    last_error      TEXT,                           -- short diagnostic on failure
    event_count     INTEGER NOT NULL                -- events from the last good sync
) STRICT;

-- Idempotent upsert key for a feed's events. Partial so only external events
-- (with a non-NULL calendar id) participate; native events are unaffected.
CREATE UNIQUE INDEX idx_events_source_unique
    ON events (source_calendar_id, source_external_id)
    WHERE source_calendar_id IS NOT NULL;
