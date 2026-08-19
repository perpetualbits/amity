# Amity — Project Status Report

*Living handoff document. Last updated 2026-08-19 at `main` = `59a612a`.*
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
  **Deliberately outside the workspace** — needs WebKit2GTK, absent on the dev
  machine, so it can only `vite build` (type-check), never run live here.

**ADRs:** `0001` initial architecture · `0002` recurrence engine · `0003`
deferred task fields · `0004` external calendar ingestion.

## What's been built (Tasks 1–5, all shipped to `main`)

| Task | Area | State |
|---|---|---|
| **1** | Scaffolding + **Inbox** capture pipeline | done |
| **2** | **Task** entity (effort/priority/recurrence, builder) | done |
| **3** | **Surfacing** + **Today** view (`rank_today`, kind-agnostic ranking) | done |
| **4** | **Event** entity + calendar surfacing on Today | done |
| **5** | **ICS ingestion & external calendars** (read-only aggregation) | done |

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

- **`main` = `59a612a`, in sync with `origin/main`.** Clean tree.
- **174 tests passing** across 19 suites; **`cargo fmt` clean; `clippy -W
  clippy::pedantic` 0 warnings; comment-density gate 0 failures.**
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
- **P3** Calendar aggregation + EventOverride + recurrence — done *(except: only
  the **Cancel** override is wired to surfacing; **Reschedule/Annotate** are not)*
- **P4** Tasks + recurrence + CompletionLog + chores — core done *(chore views
  partial)*
- **P5** Notifications (three-level) + hub-at-rest + LED — not built
- **P6** Polish + accessibility + pilot — not built

## Known carry-overs / deferred items (none blocking)

- **Event overrides:** only `Cancel` is applied to surfacing;
  `Reschedule`/`Annotate` are unwired.
- **Hub frontend has no JS test runner**; and it can't run live here (no
  WebKit2GTK) — only `vite build`.
- Optional **ICS Task 7** (read-only Calendars list in the hub) was not built.
- `calendar_sync`'s per-event sweep + batched upsert **aren't wrapped in one
  transaction** (self-healing next 6 h cycle).
- `SyncReport.calendars_synced` counts **attempts, not successes**; no direct
  test for `delete_calendar`'s instance cascade (code verified by inspection).

## Open decision: what is Task 6?

No successor is defined anywhere in the repo (`project-map.js` itself notes
this), so the roadmap's "next" marker is currently **empty**. Strongest
candidates:

1. **P2 · Meals, Lists & Pantry** — the skipped early phase; central to the MVP
   "definition of done"; largest chunk.
2. **Finish event overrides** (Reschedule/Annotate → surfacing) — completes P3;
   small.
3. **P5 · Notifications + hub-at-rest** — later in the sketch but a substantial
   phase.
4. **Hub Calendars list** (optional ICS Task 7) — small; rounds out the ICS UX.

Once Claude.ai picks, the Claude Code prompt should: name the task + acceptance
criteria, point at the relevant existing patterns to mirror, restate the
guardrails above, and (for a multi-slice feature) ask for a brief plan first.
Claude Code then sets the `project-map.js` "next" marker as part of that work.
