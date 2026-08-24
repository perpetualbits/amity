# Amity — Task 9: Members & small seams

*Brief prepared by Claude.ai, 2026-08-24, against main = 6a92d75. A deliberately small task: two seams the live prototype revealed. Resist growth; if a slice starts wanting to be bigger, that's a flag, not an invitation.*

## Why this task

Two findings from the P2 report, both small, both high-value, both prerequisites for a clean Task 7:
1. Cook and assignee render as UUIDs everywhere a person appears (Menu, Today, task views). A minimal member registry unblocks name display across the app.
2. The one-list + suppression model has a cross-week seam: a checked-off staple still on the list blocks its own re-addition next week. A one-tap "clear checked items" smooths the weekly cycle.

## Design boundary: what a Member is — and is refused to be

A Member is a **display registry entry**: display name (required), optional short initial/label, optional color token for visual distinction. That is the entire entity.

Explicitly refused, now and as a standing boundary for future phases: accounts, authentication, passwords, per-member devices or sessions, roles/permissions, activity tracking, presence, statistics, or any per-member behavioral data. Children are members exactly like adults — no age field, no child-specific structure. If a future feature wants to hang something off Member, it argues for it against this paragraph first. (This mirrors the surveillance commitments in the philosophy: a people table is where surveillance features would take root, so the entity is kept too small to host them.)

## Scope, in slices

### Slice 1 — Member entity, end to end
- amity-core: Member entity + builder (display_name required non-empty, optional initial, optional color from a small fixed token set). Mirror existing entity patterns.
- amity-storage: migration 0006 (established conventions), members repository.
- amity-service: CRUD API module mirroring existing shapes. **Management is API-only for now** — no hub settings view; a household of 3–6 changes membership approximately never, and a settings surface is Task-something-later. Extend seed-demo.sh with a few members.
- Existing cook/assignee UUID fields now reference members. In the plan, state how dangling references are handled (recommended: render a neutral fallback like "—" rather than erroring; no destructive migration of existing data).

### Slice 2 — Names in the hub
- Everywhere a person UUID currently renders (Menu cook, Today items, task assignee), resolve and display the member's name (+ color/initial where the design benefits).
- Where a cook/assignee is *set* in existing hub flows (the meal capture flow), replace free-UUID entry with a picker over registered members, including a "no one" option. Touch-first standards as established.
- No new views. No member management UI.

### Slice 3 — Clear checked items
- An explicit one-tap action on the Groceries view: remove all checked items from the active list. Confirm before executing (a tap-slip that silently deletes the record of what was bought is the wrong kind of surprise). Manual only — no scheduled or automatic clearing; the humans own the list's lifecycle.
- Core logic pure and tested, including the interaction that motivated it: after clearing, regeneration may re-add a previously-bought line (pantry suppression still applies).

### Slice 4 — Housekeeping
- project-map.js + PROJECT_STATUS.md synced; "next" marker stays Task 7 · at-rest UI + weather (marked, not scoped — Claude.ai scopes it next, with the frontend-design skill and the weather-egress posture noted in PROJECT_STATUS as forthcoming).

## Required in the result

Branch/commit, gate summary, screenshots: Menu with real cook names, the member picker, the clear-checked flow (before/confirm/after), and a regeneration-after-clear demonstrating re-addition. Anything deferred.

## Out of scope

Member management UI, per-member filtering or views ("Alice's tasks"), avatars/photos, any field beyond name/initial/color, notification targeting, at-rest UI, weather.

## Guardrails (unchanged)

Plan-first with a short plan and pause for confirmation (short task, short plan). Per-slice review + final whole-branch review. TDD throughout. amity-core I/O-free, clock injected. Comment density ≥ 50% on production crates/**/*.rs. fmt / clippy-pedantic 0 warnings / full tests / density gate green before every commit. Conventional Commits + DCO (`git commit -s`, multi-line via `git commit -F`). Feature branch; merge+push only after review-clean and maintainer approval.
