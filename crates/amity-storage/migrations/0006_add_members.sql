-- 0006_add_members.sql — flesh out the members table (Task 9 Slice 1).
--
-- Backs amity_core::member::Member — a display-registry entry only: a name,
-- an optional single-letter initial, and an optional accent colour. See the
-- module doc on amity_core::member for the hard boundary on what this table
-- is (and is not) allowed to grow: no accounts, roles, activity, or
-- age/child-specific columns.
--
-- Migration 0001 already created a MINIMAL STUB `members` table (id-only,
-- IF NOT EXISTS) purely to satisfy FK constraints from inbox_items/tasks/etc.
-- before the real Member entity existed, and inserted one placeholder row
-- (UUID 00000000-0000-7000-8000-000000000001). This migration cannot
-- CREATE TABLE members again — it must ALTER the existing stub in place so
-- that row (and its id, still referenced by every FK across the schema)
-- survives untouched.
--
-- SQLite requires a non-NULL DEFAULT on any NOT NULL column added via ALTER
-- TABLE ADD COLUMN to a non-empty table, so the two required columns get a
-- placeholder default below — applied ONLY to the pre-existing stub row.
-- Every column written by amity-storage::member from here on supplies real
-- values explicitly (see insert_member); the defaults are purely to satisfy
-- the constraint for that one legacy row and are never relied on by the app.
--
-- Existing MemberId values referenced elsewhere (Task.owner_id/assignee_ids,
-- Meal.cook_id, …) may have no matching row here at all — that is expected
-- and NOT migrated; the hub renders an unresolved id neutrally (Slice 2).

ALTER TABLE members ADD COLUMN display_name TEXT NOT NULL DEFAULT 'Unnamed member';
ALTER TABLE members ADD COLUMN initial      TEXT;                       -- optional, NULL if none
ALTER TABLE members ADD COLUMN color        TEXT;                       -- snake_case MemberColor, NULL if none
ALTER TABLE members ADD COLUMN created_at   TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';
