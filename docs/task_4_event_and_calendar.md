# Task 4 — Event entity, calendar aggregation, and surfacing integration

*Claude Code task description. Read in full before starting; surface questions before writing code.*

---

## Context

Task 3 shipped the surfacing layer behind a `SurfacedKind` enum whose only
variant today is `Task`. That enum is a seam — a promise that Today can show a
mixed-type list. Task 4 makes it real by adding the **second surfacable type,
Event**, so the hub's Today view shows what's *happening* today (the school
run, bin day, a dentist appointment) beside what needs *doing*.

Amity is a calendar **aggregator**, not a source of truth (brief §7): external
calendars own their events; the hub displays them and adds a small native
calendar for family-coordination events that belong nowhere else. Task 4
introduces the Event entity, the `EventOverride` overlay for editing read-only
external instances, and — the payoff — the surfacing integration that puts
events on Today.

Before writing any code, re-read:

- `docs/amity_brief.md` — §6.5 (Event / EventOverride / Presence), §7 (calendars
  and time semantics), §16 (internationalization / Dutch time).
- `docs/adrs/0002-recurrence-engine.md` — the recurrence engine Task 4 reuses.
- `crates/amity-core/src/surfacing.rs` — the `SurfacedKind` / `SurfaceCandidate`
  seam this task extends.
- `docs/amity_philosophy.md` — surfacing-as-the-hard-part; tone as a property of
  the surfacing layer.

## What Task 4 delivers

The point of the task is the surfacing integration; the entity and storage exist
to feed it.

### 1. The `Event` entity

The native Event per brief §6.5, with a deliberate live/deferred field split:

- `id: EventId`, `title`, `start_at`, `end_at`, `all_day`, `timezone`.
- `location: Option<String>`.
- `recurrence: Option<RecurrenceRule>` — native recurring events reuse the
  Task 2 engine (RRULE subset, DST-anchored materialisation).
- `source: EventSource { kind: Native | Ics, external_id?, calendar_id?,
  read_only, last_synced_at? }` — the field that decides edit affordances:
  full control for native events, override-only for read-only external ones.
- **Deferred (columns/types stubbed):** `attendees`, `reminders` — they interact
  with member management and the notification system, neither of which exists.

### 2. `EventOverride`

A local overlay applied to instances of read-only external events, so the
household can record "bin day moved for King's Day" without writing back to the
source: `{ source_event_id, instance_date, action: Cancel | Reschedule |
Annotate, payload, created_by, created_at }`.

### 3. Storage

Migration `0003_add_events.sql` — `events`, `event_instances` (materialised
recurring instances, mirroring `task_instances`), and `event_overrides`.
Repository modules with the same shape as the Task repositories.

### 4. HTTP API

Native-event CRUD plus the surfacing feed:

- `POST /api/v1/events` — create a native event (materialise instances if recurring).
- `GET /api/v1/events` / `GET /api/v1/events/{id}` — list / fetch.
- `PATCH /api/v1/events/{id}` — edit a native event (422 on a read-only source).
- `POST /api/v1/events/{id}/override` — create an `EventOverride` on an instance.

### 5. Surfacing integration (the payoff)

Extend surfacing so events land on Today alongside tasks:

- `SurfacedKind::Event` becomes a real variant.
- The `SurfaceCandidate` liveness signal is generalised beyond `TaskStatus` (an
  event has no Open/Doing/Done — it simply happens at a time), so the pure rule
  stays honest for both kinds. This is the main design decision in the task
  (see Open Questions).
- The `/surfacing/today` handler gathers event instances/one-offs for the day
  and feeds them in; the ranking (by time, then a kind-appropriate tiebreak)
  already handles a mixed list.
- `EventOverride`s (cancel / reschedule) are applied before surfacing so a
  cancelled instance does not appear.

### 6. Frontend

The Today view renders events with a small kind marker (position + label, not
colour alone) so a household member can tell "happening" from "to do" at a
glance. All-day events read distinctly from timed ones. No calendar grid.

## Open Questions

These need maintainer input before commitment.

- **ICS aggregation — in Task 4 or Task 5?** The recommendation is **native
  events + `EventOverride` + surfacing in Task 4, and read-only ICS ingestion
  (fetch, parse, sync a feed) as Task 5** — ICS fetching/parsing/scheduling is a
  substantial sub-project (HTTP fetch, an ICS parser dependency, a sync job) and
  bundling it risks a 12+ day task. The `EventSource::Ics` variant and
  `read_only` flag land now so the schema and override path are ready; the
  ingestion that populates them comes next. **This is the biggest fork — please
  confirm.**
- **Generalising `SurfaceCandidate` liveness.** Events have no `TaskStatus`.
  Proposed: replace the `status: TaskStatus` field with a small `liveness` the
  caller sets (a task maps Open/Doing→live, Done/Skipped→settled; an event is
  always live until it has passed). This keeps `rank_today` kind-agnostic.
  Confirm the approach.
- **How events surface.** Proposed: an event surfaces on the date of its
  `start_at`; its salient time is `start_at`; events are never "overdue"
  (they happen or they have passed — a passed event drops off, it is not nagged).
  Confirm.
- **All-day events.** Proposed: surface on their date, sorted before timed
  events that day, shown as "all day". Confirm.
- **Native event recurrence.** Reuse the Task RRULE subset + materialiser as-is?
  (Recommended — one engine, already tested.)

## Deliverables (concrete)

### `amity-core`
- `event.rs` (`Event`, `EventSource`, `EventSourceKind`, builder, errors),
  `event_override.rs`, `EventId` in `ids.rs`.
- `surfacing.rs` change: `SurfacedKind::Event` and the generalised liveness.
- Unit tests: Event construction/validation, recurrence reuse, and surfacing
  ranking over a **mixed** task+event set.

### `amity-storage`
- Migration `0003_add_events.sql`; repositories for events, event instances,
  and overrides. Integration tests for the insert-materialise-override cycle.

### `amity-service`
- `api/event.rs` (CRUD + override); the surfacing handler extended to gather
  event candidates and apply overrides. Integration tests: an event created via
  the API appears on `/surfacing/today`; a cancelled instance does not.

### `apps/hub-tauri`
- Today view renders events with a kind marker; a Tauri `create_event` command.
  (Type-checked/bundled here; live run pending WebKit, as in Task 3.)

### Documentation
- ADR `0004-event-and-calendar.md` — the aggregator posture, the native/ICS
  source split, the surfacing-liveness generalisation, and (if ICS is deferred)
  the Task 4/5 boundary.

## Acceptance criteria

Same baseline as Tasks 1–3 (`cargo build`/`test`/`clippy -W clippy::pedantic`/
`fmt --check`/`doc`; 50% comment density on new `crates/**` files; DCO-signed
Conventional Commits; ADR lands), plus:

- [ ] Unit tests: Event validation, recurrence reuse, and a **mixed task+event**
      surfacing test proving both kinds rank together correctly.
- [ ] Integration test: an event created via the API surfaces on `/surfacing/today`.
- [ ] Integration test: an `EventOverride` cancel removes an instance from Today.
- [ ] Manual: create a recurring native event; verify instances materialise and
      surface; override one instance; verify the override takes effect.

## Scope guardrails

This task does **not** include:

- ICS ingestion (fetch/parse/sync) — deferred to Task 5 unless the Open Question
  above moves it in.
- Google/Apple OAuth — external calendars are read-only ICS only, ever.
- A calendar grid / month / week view — events surface on Today; rich calendar
  rendering is a later task.
- Write-back to any external source — the whole point of `EventOverride`.
- Attendees / reminders behaviour — columns stubbed; they need members and
  notifications.
- Presence — a separate entity and task.

If work creeps past these, stop and ask.

## Estimated effort

**6–9 focused days** for the native-events-plus-surfacing scope. If ICS
ingestion is pulled in, add 4–6 days and split it out mid-task.

## Reading order suggestion

1. Brief §7 (calendars & time) and §6.5 (Event / EventOverride).
2. ADR-0002 (the recurrence engine being reused).
3. `crates/amity-core/src/surfacing.rs` and `crates/amity-service/src/api/surfacing.rs`
   (the seam and handler being extended).
4. This task description. Then plan, confirm the Open Questions, implement.

---

*The measure of this task is the first morning the hub shows a bin-day event and
a "water the plants" task in the same calm list, ordered by the clock.*
