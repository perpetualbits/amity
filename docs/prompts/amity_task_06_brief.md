# Amity — Task 6: Make the prototype visible (hub runs live, override completion, Week view)

*Brief prepared by Claude.ai, 2026-08-19, against main = 59a612a. Claude Code executes; report back with a short result and any open questions.*

## Why this task

Five tasks in, the service is well-built and well-tested, but no one has yet seen the hub on a screen — `apps/hub-tauri` has only ever been `vite build`-checked because the Tauri Linux prerequisites are not installed on this machine. The prototype-first plan depends on a tablet in a kitchen teaching us what the design got wrong; that loop can't open until the hub runs. This task opens it.

Prototype path order, for orientation: scaffolding + Inbox → entities end-to-end → **Today and Week views** → weather + at-rest UI → meals/lists. Today exists; Week is the gap. Meals come *after* the at-rest UI, not before — do not drift there.

## Scope (in order — each slice green before the next)

### Slice 0 — Hub runs live on this laptop

- Install the Tauri v2 Linux prerequisites (WebKit2GTK 4.1, librsvg, libappindicator/ayatana, build-essential, etc. per current Tauri docs). This is an environment step; if `sudo` or a package is unavailable, **stop and report** rather than working around it.
- Confirm `cargo tauri dev` (or the project's equivalent) launches the hub against a running `amity-service` on loopback, and that the Capture and Today views render and respond to touch on the laptop's touchscreen.
- Add a single documented launch path (script or `justfile`/`Makefile` target — mirror whatever the repo already uses) that starts the service and the hub together for local use. Fullscreen/kiosk flag behind an env var or CLI switch; default off.
- Record the prerequisite list in the repo (README or `docs/`), so this never has to be rediscovered.

No JS test runner exists yet. Do not introduce one in this task unless it is trivial (e.g. vitest with one smoke test); if you do, keep it to the smallest footprint and note it in the result.

### Slice 1 — Finish event overrides in surfacing

Only `Cancel` currently reaches surfacing. Wire `Reschedule` and `Annotate` through the same path, mirroring how `Cancel` is applied to the instance stream (external events included — remember the Task 5 seam bug; external events route through the instance path).

- Reschedule: the surfaced instance carries the overridden start/end; the original slot does not also surface.
- Annotate: the surfaced instance carries the annotation; timing unchanged.
- e2e tests for each, plus one covering an override on a recurring **external** event to lock the seam.

### Slice 2 — Week view

**Core.** A pure function in `amity-core` (alongside `rank_today`, same clock-injection discipline) that, given a 7-day window starting at a caller-supplied date, returns per-day buckets of surfaced items: events (native + external, overrides applied) and tasks with a date in-window. Reuse the existing instance/surfacing machinery; do not duplicate it. Week starts Monday. Ordering within a day: all-day first, then by start time, then tasks. No scoring/ranking beyond that — Week is a layout of what *is*, not a prioritisation.

**Service.** One endpoint (mirror the Today endpoint's shape and error handling), e.g. `GET /week?start=YYYY-MM-DD`, defaulting to the current week. Loopback only, as everything else.

**Hub.** A `Week` view beside `Today` and `Capture`:
- Seven columns (or a stacked layout in portrait — pick one, note the choice); today visually marked.
- Touch-first: minimum 44 px targets, no hover-only affordances, no right-click.
- Per-item: title, time (or "all day"), source marker for external-calendar events, annotation if any, rescheduled marker if any.
- Read-mostly: tapping an item may show detail; there is **no** editing, dragging, or creating from the grid. New things go through the existing Capture flow.
- Prev/next week navigation; a "this week" return.
- Overflow: if a day exceeds what fits, truncate with an "and N more" affordance that expands — do not shrink text below legibility.

Verify the whole thing visually, on the touchscreen, against seeded data that includes: an external recurring event with an EXDATE, a rescheduled event, an annotated event, an all-day event, and a dated task.

### Slice 3 — Housekeeping

- Set `project-map.js` "next" marker appropriately (this task while in progress; on completion, point it at Task 7: *hub at-rest UI + weather* — do not scope Task 7, just mark it).
- Read-only check, report in the result, change nothing: does the Task 4 chores implementation contain any rotation/assignment *algorithm* (automatic assignment of chores to people), as opposed to recurrence plus optional manual assignee? The design stance is "planner, not mediator"; if an algorithm exists, describe it in two or three sentences and stop.

## Out of scope (do not drift)

At-rest UI, weather, meals/lists/pantry, notifications/LED, ICS Calendars list in the hub (optional Task 7 from ICS work stays deferred), mobile companion, event editing from the Week view, any new ADR unless a real architectural decision is forced (then propose it in the plan first).

## Acceptance criteria

1. The hub launches and renders live on this laptop via a single documented command; Capture, Today, Week respond to touch.
2. Reschedule and Annotate overrides are applied in surfacing, with e2e coverage including the external-recurring case.
3. `GET /week` returns Monday-start 7-day buckets with overrides applied; core function is pure and clock-injected.
4. Week view renders the seeded scenarios correctly on screen; no edit/drag/create in the grid.
5. All gates green: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -W clippy::pedantic` (0 warnings), `cargo test --workspace`, comment-density gate 0 failures; `vite build` clean.
6. `project-map.js` updated; prerequisites documented; result note includes the chore-rotation check.

## Process & guardrails (unchanged from prior tasks)

- **Plan first.** Before implementing, produce a short plan (slices → sub-tasks, files touched, tests to write) and pause for confirmation.
- Subagent-driven: one implementer per slice, per-slice review, **final whole-branch review** before merge — it has caught cross-cutting bugs every time.
- TDD mandatory: failing test → watch it fail → minimal code → pass.
- `amity-core` stays I/O-free; clock always injected.
- Comment density ≥ 50 % on production `crates/**/*.rs`.
- Conventional Commits + DCO (`git commit -s`); multi-line/backtick messages via `git commit -F <file>`.
- Feature branch; merge to main and push only after review-clean and maintainer approval.

## Open questions for the maintainer (answer before or during planning)

- Portrait or landscape for the wall station? (Affects Week column vs. stacked layout; if unknown, build landscape and keep the layout switchable.)
- Should Week show *all* dated tasks, or only those not yet done? (Default: only open ones.)
- Any existing styling tokens/components in the hub to reuse for the grid, or is Week the first "layout-heavy" view?

## What to report back

Branch/commit, gate output summary, a screenshot or two of the Week view on the touchscreen, the chore-rotation check, anything deferred, and anything that felt like a design decision rather than an implementation one.
