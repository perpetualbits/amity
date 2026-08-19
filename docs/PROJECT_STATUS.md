# Amity — Project Status Report

*Living handoff document. Last updated 2026-08-20 (Task 6 backend merged; hub half blocked; **P2 is next**).*
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
**Today** view, tasks & chores with fair rotation, an aggregated calendar, and
(planned) meals/lists/notifications. Design ethos: **information over pressure**,
privacy-first, no surveillance or commercial data flow. Full brief in
`docs/amity_brief.md`; philosophy in `docs/amity_philosophy.md`.

## Architecture & stack

Rust **Cargo workspace**, three crates with a strict dependency direction:

- **`amity-core`** (~5,770 LOC) — pure domain, **no I/O** (no tokio/sqlx/reqwest).
  Entities, builders, the recurrence engine, ICS parsing, and the
  surfacing/ranking logic. Clock is always injected (`now: OffsetDateTime`).
- **`amity-storage`** (~3,250 LOC) — `sqlx` + **SQLite STRICT**; migrations
  `0001`–`0004`; one repository module per entity. RFC-3339 TEXT datetimes, TEXT
  UUID-v7 ids, INTEGER 0/1 bools, enums via `Display`/`FromStr` snake_case.
- **`amity-service`** (~4,590 LOC) — `axum` HTTP API (one module per entity),
  background **jobs** (`recurrence_horizon`, `calendar_sync`), and `feeds` (the
  one outbound egress). Loopback-only.
- **`apps/hub-tauri`** — SolidJS + Tauri v2 frontend (`Capture`, `Today` views).
  **Deliberately outside the workspace** (its own `[workspace]` root as of Task 6).
  WebKit2GTK **is now installed** (2.52.3), and `vite build` (frontend) passes —
  but the **native Tauri side does not compile** (see the blocker below), so the
  hub still cannot run live here. It had only ever been `vite build`-checked, which
  hid this.

**ADRs:** `0001` initial architecture · `0002` recurrence engine · `0003`
deferred task fields · `0004` external calendar ingestion.

## What's been built (Tasks 1–5 on `main`; Task 6 backend on branch `task-6-hub-live-week`)

| Task | Area | State |
|---|---|---|
| **1** | Scaffolding + **Inbox** capture pipeline | done |
| **2** | **Task** entity (effort/priority/recurrence, builder) | done |
| **3** | **Surfacing** + **Today** view (`rank_today`, kind-agnostic ranking) | done |
| **4** | **Event** entity + calendar surfacing on Today | done |
| **5** | **ICS ingestion & external calendars** (read-only aggregation) | done |
| **6** | Event **overrides** in surfacing + **Week view backend** | **backend done; hub UI blocked** |

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
mediator"). **Blocked (Slices 0 + 2b):** the hub's native side won't compile —
see the blocker below — so the live prototype and the Week **UI** are deferred to
a follow-up. The Week **backend** is ready and tested behind `/api/v1/week`.

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

- **`main` = `b3ffe5c`, in sync with `origin/main`.** Task 6 backend merged
  (Slices 1 + 2a + the hub `[workspace]` fix). Clean tree.
- **197 tests passing**; **`cargo fmt` clean; `clippy -W clippy::pedantic` 0
  warnings; comment-density gate 0 failures.** (Backend only — the hub native
  build is a separate, blocked concern; see the blocker.)
- Migrations `0001`–`0004`; a live `project-map.js` at repo root tracks status
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

## ⛔ Hub build blocker (Task 6 — the reason the hub half is deferred)

The hub's **native (Tauri) side does not compile on this machine**, and never had
(it was only ever `vite build`-checked). Bringing it up for Task 6 surfaced:

- **Fixed:** `src-tauri` was silently pulled into the amity Cargo workspace →
  gave it its own empty `[workspace]` root (commit on the branch).
- **Blocking (upstream):** `tauri-macros 2.6.3` (the latest — `cargo update` finds
  nothing newer) generates a `#[tauri::command]` helper as `macro_rules! X; use X;`
  in one module, which rustc rejects as **E0255 "defined multiple times."** It
  fails on **every rustc from 1.88 to 1.95** (deps need ≥1.88; the E0255 is present
  by 1.90), so no toolchain in range works. Edition 2021↔2024 makes no difference.
  A bounded Tauri **downgrade** attempt cascaded into a `tauri-build`/`tauri-utils`
  mismatch — a full `tauri-*`-family pin would be needed.

**To unblock later (a follow-up task):** wait for an upstream `tauri-macros`
release, or pin the whole `tauri-*` family to a consistent older set on stable
rustc, then verify `cargo build` in `apps/hub-tauri/src-tauri`, add a `run-hub`
launch script + prereq doc (Ubuntu deps: `libwebkit2gtk-4.1-dev build-essential
curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`),
and build the Week **UI** against the ready `/api/v1/week` endpoint.

## Known carry-overs / deferred items (none blocking)

- **Hub live prototype + Week UI** — deferred by the blocker above (the roadmap
  "next" marker points here).
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

## Next task: P2 · Meals, Lists & Pantry (decided 2026-08-20)

The maintainer chose **P2 — Meals, Lists & Pantry** (the meal→groceries pipeline)
as the next task: pure backend, so it entirely sidesteps the blocked hub. The
roadmap "next" marker points here. The hub live-prototype + Week UI follow-up
stays deferred (blocked upstream) until Tauri is fixed.

**P2 scope, from the brief** (§18.1 Weeks 3–4; for Claude.ai to turn into a task
brief): Meals + Lists + PantryItem entities, and the meal-to-groceries pipeline
end-to-end *without recipes* (per §18.1). MVP definition-of-done touchpoints
(§18.3): "Menu planned, groceries generated and checked off on mobile." Mirror the
established entity pattern (builder + clock injection in `amity-core`, a
migration + repository in `amity-storage`, an axum API module in `amity-service`,
subagent-driven with TDD and the gates below).

**Awaiting the P2 task brief from Claude.ai** (drop it in `docs/prompts/`, same as
the Task 6 brief). A good Claude Code prompt: names the task + acceptance
criteria, points at the entity patterns to mirror (`task.rs`/`event.rs` +
their storage/API), restates the guardrails above, and asks for a brief plan
first. Claude Code sets the `project-map.js` "next" marker as part of the work.
