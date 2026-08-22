# Amity — Project Status Report

*Living handoff document. Last updated 2026-08-22 (Task 8 / P2 Meals-Lists-Pantry done; **next: Task 7 · at-rest UI**).*
*Update this file whenever a task lands (it is the source of truth Claude.ai reads to prepare the next prompt).*

## Working model (henceforth)

**Claude.ai leads; Claude Code executes.** Claude.ai holds the durable project
context (description, roadmap, decisions) and writes each work prompt; Claude
Code runs that prompt in the repo and reports back. This avoids Claude Code
degrading once its context fills.

Each cycle: Claude.ai issues a scoped task prompt → Claude Code implements +
tests + commits → Claude Code returns a short result → Claude.ai decides the
next task and updates its project context. Keep this file current so the loop
survives a fresh Claude Code session.

## What Amity is

A **local-first household planner** ("a peaceful home") — a Rust service + tablet
"hub" that helps a family of 3–6 run their week: capture-to-inbox, a ranked
**Today** view, tasks & chores (recurrence + a *manual* assignee — never
auto-rotated; planner, not mediator), an aggregated calendar, meal planning with
a grocery pipeline, and (planned) notifications. Design ethos: **information over
pressure**,
privacy-first, no surveillance or commercial data flow. Full brief in
`docs/amity_brief.md`; philosophy in `docs/amity_philosophy.md`.

## Architecture & stack

Rust **Cargo workspace**, three crates with a strict dependency direction:

- **`amity-core`** (~5,770 LOC) — pure domain, **no I/O** (no tokio/sqlx/reqwest).
  Entities, builders, the recurrence engine, ICS parsing, and the
  surfacing/ranking logic. Clock is always injected (`now: OffsetDateTime`).
- **`amity-storage`** — `sqlx` + **SQLite STRICT**; migrations `0001`–`0005`; one
  repository module per entity. RFC-3339 TEXT datetimes, TEXT UUID-v7 ids,
  INTEGER 0/1 bools, enums via `Display`/`FromStr` snake_case.
- **`amity-service`** — `axum` HTTP API (one module per entity: inbox, task,
  event, calendar, meal, grocery, pantry, surfacing), background **jobs**
  (`recurrence_horizon`, `calendar_sync`), and `feeds` (the one outbound egress).
  Loopback-only.
- **`apps/hub-tauri`** — SolidJS + Tauri v2 frontend, five views (Today, Week,
  Capture, Menu, Groceries). **Deliberately outside the workspace** (its own
  `[workspace]` root). WebKit2GTK installed; the hub **builds and runs live** via
  `scripts/run-hub.sh` (Tauri commands must stay non-`pub` — see the hub README).

**ADRs:** `0001` initial architecture · `0002` recurrence engine · `0003`
deferred task fields · `0004` external calendar ingestion.

## What's been built (Tasks 1–8 — all on `main` once P2 merges)

| Task | Area | State |
|---|---|---|
| **1** | Scaffolding + **Inbox** capture pipeline | done |
| **2** | **Task** entity (effort/priority/recurrence, builder) | done |
| **3** | **Surfacing** + **Today** view (`rank_today`, kind-agnostic ranking) | done |
| **4** | **Event** entity + calendar surfacing on Today | done |
| **5** | **ICS ingestion & external calendars** (read-only aggregation) | done |
| **6** | Event **overrides** in surfacing + **Week view backend** | done |
| **6b** | Unblock the hub, run it live, ship the **Week UI** | done — **hub runs live** |
| **8 / P2** | **Meals, Lists & Pantry** (+ grocery generation, meal on Today) | done |

**Task 8 / P2 detail** (most recent): `Meal` (date, slot, name, optional cook,
optional freetext ingredient lines — **no recipes**), `GroceryList`/`GroceryItem`,
and a **lightweight `PantryItem`** (staples that *suppress* generation — no
levels/thresholds); a pure `plan_grocery_additions` with a proven **no-clobber**
regenerate property; migration 0005; CRUD APIs + a `POST /grocery-lists/{id}/generate`
endpoint; **Menu** + **Groceries** hub views (run live, maintainer-accepted); and
tonight's dinner surfacing on **Today** as an informational `SurfacedKind::Meal`
(not on Week). Known limitation: **cook displays as an id/"cook assigned", not a
name** — no member-name registry yet (the `people` entity is still a placeholder).

**Task 6 detail** (most recent): **Slice 1** — `Reschedule` and `Annotate`
overrides now apply in surfacing (only `Cancel` did before), on the shared
instance path so external/recurring events are covered; guarded by e2e incl. the
external-recurring seam. **Slice 2a** — a pure, clock-injected `plan_week`
planner (Monday-start 7-day buckets, overrides applied, open tasks only, layout
not ranking) + `GET /api/v1/week?start=` endpoint; `candidate_to_item` factored
and shared with `rank_today`. Both review-clean. **Chore-rotation check (Slice 3):**
confirmed there is **no automatic rotation/assignment algorithm** — chores are
recurrence + an *optional manual* assignee (instances inherit the parent's, per-
instance overridable); the code explicitly disavows auto-fairness ("planner, not
mediator").

**Task 6b** then **unblocked the hub and shipped the Week UI** — the hub now
builds and runs live (`scripts/run-hub.sh`), with the Week grid visually
verified by the maintainer. See "How the hub was unblocked" below (my earlier
"upstream tauri-macros" diagnosis was wrong — the real cause was local).

**Task 5 detail** (most recent): a `Calendar` entity; pure iCalendar
`parse_feed`/`expand_external` (via `ical` + `rrule` crates, DST-correct);
migration `0004` + calendars repo + idempotent external-event upsert/prune; the
system's **first outbound egress** `feeds::fetch` with guards (scheme allow-list,
20 s timeout, cumulative 5 MiB cap, 5-redirect bound, no-compression build); a
6-hour `calendar_sync` job; a calendars CRUD API; and ADR-0004. Built
subagent-driven with a per-task review loop plus a **final whole-branch review
that caught a Critical seam bug** (recurring feed events surfaced only on their
first occurrence — surfacing read materialised instances only when
`recurrence.is_some()`, but external events keep `recurrence = None`) — fixed by
routing external events through the instance path, plus a stale-instance sweep on
re-sync, guarded by a new recurring+EXDATE e2e. HTTP-level `wiremock` tests for
the egress guards were then added (mutation-verified to bite).

## Repository health (as of this update)

- **Task 8 / P2 on branch `task-p2-meals`** (5 slices: entities, storage,
  service, hub, housekeeping), merging after final review; `main` was at `a08aff5`
  (Task 6b).
- **Workspace tests all green**; **`cargo fmt` clean; `clippy -W clippy::pedantic`
  0 warnings; comment-density gate 0 failures.** The hub (`apps/hub-tauri`,
  outside the workspace) builds clean (`cargo build` + `npm run build`) and runs
  live; it has no automated tests beyond `vite build`.
- Migrations `0001`–`0005`; a live `project-map.js` at repo root tracks status
  (keep it in sync on changes).

## Engineering guardrails (Claude Code operates under these — bake into prompts)

- **TDD is mandatory** (superpowers skill): failing test first → watch it fail →
  minimal code → watch it pass.
- **Comment density ≥ 50%** on every production `crates/**/*.rs` (string-literal
  aware, excludes test code). Gate:
  `find crates -name '*.rs' | xargs bash scripts/comment-density.sh`.
- **Green before commit:** `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -W clippy::pedantic`,
  `cargo test --workspace`.
- **Commits:** Conventional Commits + **DCO sign-off** (`git commit -s`).
  Multi-line/backtick messages **must** use `git commit -F <file>` (backticks in
  `-m` execute as shell substitution — this has bitten the project).
- **`amity-core` stays I/O-free.** Clock injected, never read in core.
- **Execution style that works well here:** subagent-driven development (a plan →
  one implementer per task → task review → final whole-branch review). The final
  review has repeatedly caught cross-cutting bugs the per-task passes couldn't.
- Work on a **feature branch**, merge to `main` when review-clean, push.

## Roadmap position (brief §18.1 vs. reality)

The build has **not** followed the brief's linear week order — it did P1, P4,
then P3:

- **P1** Data model + inbox + Today — done
- **P2** Meals, Lists & Pantry (meal→groceries pipeline) — **not built;
  leapfrogged; largest unbuilt phase; core to the MVP definition-of-done**
- **P3** Calendar aggregation + EventOverride + recurrence — done *(all three
  overrides — Cancel/Reschedule/Annotate — now wired to surfacing, Task 6)*
- **P4** Tasks + recurrence + CompletionLog + chores — core done *(chore views
  partial)*
- **P5** Notifications (three-level) + hub-at-rest + LED — not built
- **P6** Polish + accessibility + pilot — not built

## How the hub was unblocked (Task 6b) — corrects an earlier misdiagnosis

The hub's native (Tauri) side had never compiled (only ever `vite build`-checked).
Task 6 attributed the failure to an **upstream `tauri-macros` bug** on rustc
1.88–1.95. **That was wrong.** The real cause, per current Tauri v2 docs, is a
long-standing, version-independent property of `#[tauri::command]`: a command
function marked **`pub` and colocated in the same file as `tauri::generate_handler!`**
collides with its own generated helper macro (**E0255**). Task 6b's fix, on the
current toolchain and latest Tauri (no version changes):

- **Dropped `pub`** from all six `#[tauri::command]` fns in `src-tauri/src/lib.rs`
  → all 13 E0255 errors gone. (They must stay non-`pub`; see the hub README.)
- Provided **placeholder app icons** (`generate_context!` needs them) and wired
  `tauri.conf.json`.
- Fixed `tauri.conf.json` to use **npm** (not `pnpm`) for its dev/build commands.
- Kept the earlier, legitimate **`[workspace]`** fix (src-tauri as its own root).

The hub now builds and runs live via **`scripts/run-hub.sh`** (starts
`amity-service` on 127.0.0.1:7890 + the hub; `AMITY_KIOSK=1` for fullscreen).
**`scripts/seed-demo.sh`** populates the current week for a visual check. Ubuntu
prereqs are in `apps/hub-tauri/README.md`. The Week grid was **visually verified**
by the maintainer.

## Known carry-overs / deferred items (none blocking)

- **Hub-at-rest UI** (clock/weather/ambient) — not built; the maintainer wants it
  eventually to be *calm but not bland* (see the aesthetic-direction note in
  Claude Code memory). This is roadmap P5 / "Task 7".
- Placeholder hub icons — swap for a real Amity icon when there is one.
- **Hub frontend has no JS test runner** — only `vite build` type-checks it.
- Optional **ICS Task 7** (read-only Calendars list in the hub) was not built.
- Minor (from Task 6 reviews): a **pre-existing** Cancel-vs-later-overlay
  precedence gap ("cancel then reschedule" still shows cancelled) — a maintainer
  decision; cross-day reschedule silently drops an instance (out of scope); the
  `/week` handler does per-day reads (N+1, fine at household scale).
- `calendar_sync`'s per-event sweep + batched upsert **aren't wrapped in one
  transaction** (self-healing next 6 h cycle).
- `SyncReport.calendars_synced` counts **attempts, not successes**; no direct
  test for `delete_calendar`'s instance cascade (code verified by inspection).

## Next task: Task 7 · hub at-rest UI + weather

P2 is done, so the roadmap "next" marker is **Task 7 — the hub-at-rest UI**
(clock / weather / ambient), which now has real content to design around (Today,
Week, and tonight's menu). This is where the maintainer's **aesthetic direction**
applies: *calm but not bland*, ambient, reflecting household activity (work,
study, building, sports) — **not busy, not in-your-face** (see the
`hub-aesthetic-direction` memory; load the `frontend-design` skill when scoping
it). A natural companion is a **member-name registry** — the `people` entity is
still a placeholder, so cook/assignee currently render as ids, not names; wiring
real member names lets Menu/Today show "Alice cooks tonight."

Other unblocked candidates if Task 7 waits: P5 · Notifications; the deferred
"structure grows with use" meal features (recipes, pantry thresholds, use-first
list); dietary-flag warnings (brief §9.3).

When Claude.ai picks, drop the task brief in `docs/prompts/`. A good Claude Code
prompt names the task + acceptance criteria, points at the patterns to mirror,
restates the guardrails, and asks for a plan first.
