# Task 3 — Surfacing layer and the Today view

*Claude Code task description. Read in full before starting; surface questions before writing code.*

---

## Context

Tasks 1 and 2 built downward: the Cargo workspace, storage, config, the HTTP
API, the **Inbox** end-to-end, and the **Task** entity with its recurrence
engine and CompletionLog — all at the backend, with ~100 tests. What the
household can actually *see* today is still only the inbox capture screen.

Task 2's backend shipped, but two things it named never landed: the **Task
capture UI** and the **recurrence horizon background job** (an explicit `TODO`
in `crates/amity-service/src/api/task.rs`). And the piece the brief calls the
hard part of the whole system — **surfacing** (§6.4) — does not exist beyond a
single `GET /api/v1/tasks/upcoming` query that returns raw instances.

Task 3 closes that gap. It turns the tested Task backend into something a
household can look at: a **Today view** fed by a real **surfacing layer**, the
**Task capture UI** to create the tasks that appear there, and the **horizon
job** so the instances behind it keep materialising. This is the first surface
where *tone* matters — surfacing, not storage, is where Amity chooses to be
information rather than pressure.

Before writing any code, re-read:

- `docs/amity_philosophy.md` — especially "the empty state is a designed state"
  and "surfacing is the hard part, not storing".
- `docs/amity_brief.md` — §3 (design principles), §6.4 (surfacing), §11.5 (hub
  at rest), §16 (Dutch date/time conventions).
- `docs/rust_guidelines.md` and `docs/coding_guidelines.md` — patterns, comment
  density, ADR discipline.
- `docs/adrs/0002-recurrence-engine.md` — the horizon/prune design this task
  finally implements.

## What Task 3 delivers

Four deliverables. The first two are the point; the third makes the first two
demonstrable; the fourth keeps them correct over time.

### 1. The surfacing layer

The single ranked query the Today view pulls from. The brief (§6.4) describes it
as spanning Event, Task, Project milestones, and Thread prompts — but only Task
exists, so this task builds surfacing **over Tasks alone**, behind a shape that
admits the other types later without a rewrite. That seam must be honest from
day one, not retrofitted.

- **A pure ranking function in `amity-core`** (new `surfacing.rs`): given a set
  of upcoming task instances, a `now`, and a small `SurfacingConfig`, return an
  ordered `Vec<SurfacedItem>` plus a flag for whether anything crossed the
  "surface today" threshold. Pure and fully unit-testable, no I/O.
- **`SurfacedItem`** — a uniform, type-tagged shape (`kind`, `id`,
  `instance_date`, `title`, `when`, `status`, `current_assignee_id`). `kind` is
  `Task` for now; the enum is the seam for `Event`/`Project`/`Thread` later.
- **Ranking inputs (only those that exist):** time proximity (the instance's
  due/earliest window relative to `now`), then `priority`, then a stable
  tiebreak. Per-person filtering, quiet hours, and Presence are **stubbed** in
  `SurfacingConfig` with `TODO`s — the member layer and Presence entity don't
  exist yet.
- **The empty state is a real result, not an error.** When nothing crosses the
  threshold the layer returns an explicit "calm" empty list. Tone is this
  layer's responsibility (§3, §11.5): an overdue-but-open task surfaces as
  information ("due: earlier today"), never with escalating colour or a count of
  how late it is.

### 2. The Today view (`apps/hub-tauri`)

The first user-facing surface beyond capture.

- A **Today** screen rendering the surfaced list: large, legible at ~1m,
  Atkinson Hyperlegible, 60×60 minimum touch targets — the same constraints as
  Task 1.
- Each item shows title, when (Dutch convention: day-first, 24-hour), and the
  current assignee (placeholder member for now).
- A **Mark done** affordance per item → `POST /api/v1/tasks/{id}/complete` with
  the item's `instance_date`; on success the item leaves Today.
- A **Reassign** affordance → `POST /api/v1/tasks/{id}/assignee`. With one
  placeholder member this is mostly scaffolding; it becomes real when members
  land. Build the affordance, not a member picker.
- The **designed empty state**: a calm "nothing today", no spinner, no skeleton,
  no placeholder cards — the same posture as the existing inbox empty state in
  `App.tsx`.
- **View switching** between Capture (inbox) and Today. The app is currently a
  single screen; introduce the smallest possible switch (see Open Questions), no
  router dependency.

### 3. The Task capture UI (`apps/hub-tauri`)

The form Task 2 specified but never shipped, so there is a way to create the
tasks the Today view displays.

- Fields: title, optional notes, optional `dueBy` date, an optional recurrence
  **preset** ("once / daily / weekly on [day] / monthly on the [nth]"), tags.
- Submits to `POST /api/v1/tasks`; clears on success, same calm feedback as
  inbox capture (no toast, no animation).
- Advanced raw-RRULE input stays deferred (as Task 2 proposed).

### 4. The recurrence horizon job (`amity-service`)

Finally implement the `TODO` from Task 2 / ADR-0002.

- A background job in `crates/amity-service/src/jobs/recurrence_horizon.rs` that
  runs **on service start and then daily**: for every recurring task, extend the
  materialised instances forward to the 60-day horizon, and prune instances that
  have aged out beyond the 30-day backward window (`prune_old_instances` already
  exists in storage).
- Without this, materialised instances stop at whatever was generated at
  create-time and the Today view silently goes empty for long-lived recurrences.

## Deliverables (concrete)

### `amity-core`

- `surfacing.rs`: `SurfacedItem`, `SurfacedKind`, `SurfacingConfig`, and
  `rank_today(instances, now, config) -> SurfacingResult { items, has_surfaced }`.
- Unit tests: ordering by time then priority; the empty-state threshold
  (nothing due → `has_surfaced == false`); overdue-but-open items still surface;
  a stable tiebreak so ordering is deterministic.

### `amity-service`

- `api/surfacing.rs`: `GET /api/v1/surfacing/today?date=YYYY-MM-DD` returning the
  ranked `SurfacedItem` list and an explicit empty result when nothing surfaces.
  (Whether to also add `/week` now — see Open Questions.)
- `jobs/recurrence_horizon.rs`: the start-plus-daily job; wire it into
  `main.rs`/`lib.rs` startup.
- Integration tests: surfacing endpoint returns ranked items and a clean empty
  result; the horizon job extends and prunes as specified.

### `apps/hub-tauri`

- A Today view component and the surfacing Tauri command(s).
- The Task capture form and its Tauri command.
- Minimal Capture/Today view switching.
- Accessibility constraints as Task 1 (Atkinson, 60×60, calm, designed empty
  states).

### Documentation

- ADR `docs/adrs/0004-surfacing-layer.md` — records the Task-only-now /
  uniform-`SurfacedItem` seam, the ranking inputs actually available vs. stubbed
  (per-person, quiet hours, Presence), and the empty-state-as-result decision.
- Note in ADR-0002 (or the new ADR) that the horizon `TODO` is now resolved.
- Update `README.md` if the run instructions change.

## Open Questions

Surface these at planning time; the maintainer decides.

- **Endpoint shape.** New `/api/v1/surfacing/today` returning uniform
  `SurfacedItem`s, or extend the existing `/tasks/upcoming`? Recommend the new
  endpoint so the mixed-type seam is honest now and `/tasks/upcoming` stays a
  raw-instances query.
- **The "surface today" rule.** Proposed: an instance surfaces if its
  due/earliest window intersects the target date, **or** it is open and overdue.
  Confirm, or refine what counts as "today".
- **Week view — in or out?** Recommend **Today only** this task; Week is a
  natural follow-up and keeps scope bounded.
- **View switching UI.** Recommend a two-item segmented control (Capture /
  Today), no routing library. Confirm.
- **Recurrence presets.** Confirm the four presets ("once / daily / weekly on
  [day] / monthly on the [nth]"); advanced RRULE deferred.
- **Horizon scheduling mechanism.** Recommend a plain `tokio` interval spawned
  at startup rather than adding a scheduler/cron dependency. Confirm.

## Acceptance criteria

Same baseline as Tasks 1–2 (`cargo build`/`test`/`clippy -W clippy::pedantic`/
`fmt --check`/`doc` clean; 50% comment density on new files; DCO-signed
Conventional Commits; ADR lands), plus:

- [ ] Surfacing ranking unit tests pass, including ordering, the empty-state
      threshold, and overdue handling.
- [ ] Integration test: the surfacing endpoint returns a ranked list and a clean
      empty result.
- [ ] Integration test: the horizon job extends the materialisation window and
      prunes aged-out instances.
- [ ] Manual: create a recurring task via the new capture UI → it appears in
      Today → mark it done → it leaves Today and a CompletionLog row is written.
- [ ] Manual: with nothing due, Today shows the calm "nothing today" empty
      state — no spinner, no nag.
- [ ] Tone check: an overdue open task surfaces as information, with no
      escalating colour and no lateness count.

## Scope guardrails

This task does **not** include:

- Real member management, Presence, or quiet-hours enforcement (all stubbed in
  `SurfacingConfig`).
- Notifications firing when a task is due — the three-level model is a later
  task; surfacing only populates a view.
- Surfacing Event / Meal / Project / Thread — Task-only now; the `SurfacedKind`
  seam is kept for them.
- The Week view (deferred), the full hub-at-rest design (clock/weather/LED),
  mobile, voice, and API auth.

If the work-in-progress creeps past these, stop and ask. If the frontend alone
starts to dominate, splitting "surfacing + Today view" from "Task capture UI +
horizon job" into two tasks is the right move — flag it.

## Estimated effort

**6–9 focused days.** Frontend-heavier than Task 2, and the first task to touch
view-switching in the Tauri app. If it runs substantially past 9 days, scope has
expanded — split it as noted above.

## Reading order suggestion

1. The philosophy on empty states and surfacing-as-the-hard-part.
2. Brief §6.4 (surfacing), §3 (empty state / tone), §11.5 (hub at rest).
3. ADR-0002 (the horizon design being implemented).
4. `crates/amity-service/src/api/task.rs` (the `upcoming` query and the horizon
   `TODO`) and `apps/hub-tauri/src/App.tsx` (the capture + empty-state pattern to
   mirror).
5. This task description. Then plan, confirm with the maintainer, implement.

---

*The measure of this task is not the code but the first morning the hub shows
"nothing today" and means it.*
