# Amity — Task 8 / P2: Meals, Lists & Pantry

*Brief prepared by Claude.ai, 2026-08-21. This is the largest phase built so far — plan-first is not a formality here. The authoritative scope is brief §18.1 and the meals/lists sections of docs/amity_brief.md, restated in docs/PROJECT_STATUS.md's P2 section; where this brief and those documents disagree on entity details, the repo documents win — but raise the disagreement in the plan rather than silently following either.*

## Why this task

The prototype replaces paper artifacts. The Week view replaced the wall calendar; this phase replaces the kitchen chalkboard: the week's menu, cook-per-day, and the grocery list that falls out of it. It is also the largest unbuilt piece of the MVP definition-of-done ("menu planned, groceries generated and checked off").

## Design commitments that bound this phase

- **No recipes.** A Meal is a planned dish for a day — a name, optionally a short list of ingredient lines. It is not a recipe database, has no instructions, no nutrition, no imports. If a slice starts wanting recipe structure, stop and flag.
- **Cook-per-day is a manual designation, never an algorithm.** A person may be *recorded* as cooking on a day (the chalkboard does this today). The system must not assign, rotate, balance, suggest, or score cook duty. Planner, not mediator. This is a hard boundary, same as chores.
- **The pipeline is generate-then-edit, not sync.** Generating a grocery list from the week's planned meals produces list items the humans then own — they add, remove, and check off freely. Re-generation must never clobber manual state (checked items stay checked; manual additions survive). Prefer explicit "add this week's meal ingredients to the list" over any continuous reconciliation.
- **Pantry is a lightweight memory, not inventory management.** PantryItem records staples the household considers on-hand so generation can skip them. No quantities-on-hand tracking, no depletion logic, no expiry. If the brief's PantryItem definition is richer than this, follow the brief — but flag it in the plan.

## Scope, in slices (each green before the next; propose refinements in the plan)

### Slice 1 — Entities (amity-core)
Meal (date, name, optional cook: person, optional ingredient lines), GroceryList / GroceryItem (name, checked, origin: generated-from-meal vs manual), PantryItem. Builders, validation, and the generation function as a **pure core function**: given a date range of meals + pantry items + the current list, produce the items to add (pantry-matched lines skipped, already-present lines not duplicated). Clock injected as everywhere. Mirror the Task/Event entity patterns.

### Slice 2 — Storage (amity-storage)
Migration 0005 (SQLite STRICT, established conventions: RFC-3339 TEXT datetimes, TEXT UUIDv7 ids, INTEGER bools, snake_case enums via Display/FromStr), one repository module per entity, mirroring existing repos.

### Slice 3 — Service (amity-service)
CRUD APIs per entity, one module each, mirroring existing endpoint shapes; a generation endpoint that applies the pure core function and persists the result. Loopback-only as always. No background jobs in this phase unless the plan argues one is genuinely needed.

### Slice 4 — Hub: the chalkboard views
Two views, same touch-first standards as Week (44 px targets, no hover, read-mostly-plus-check-off):
- **Menu**: the week's meals as a Monday-start strip — day, dish name, cook if recorded. Editing/planning a meal can open a minimal capture-style flow (name + optional cook + optional ingredient lines); keep it as small as Capture, not a form-heavy planner.
- **Groceries**: the current list, tap-to-check (the one mutation the hub does freely), manual add via the same minimal flow, and the explicit "generate from this week's menu" action.
Visual verification live on the touchscreen against seeded data: a week of meals with cooks, pantry staples that suppress lines, a regeneration after manual edits proving nothing is clobbered. Screenshots in the result.

### Slice 5 — Housekeeping
project-map.js + PROJECT_STATUS.md synced; "next" marker → Task 7 (at-rest UI + weather — now with menu content available to design around), marked, not scoped.

## Open questions — answer in the plan, ask the maintainer where genuinely ambiguous

1. Ingredient lines: bare names, or name + freetext quantity ("2 lb", "a bunch")? Default: name + optional freetext quantity string, no unit parsing, no arithmetic.
2. One rolling grocery list, or multiple named lists? Default: one active list unless the brief says otherwise.
3. Does Meal surface on Today/Week? Default: tonight's dinner (+ cook) appears on Today as an informational item, not a task; Week shows it in the day's meal slot only if the brief's Week design includes it — otherwise leave Week alone this phase.

## Required in the result

Branch/commit, gate summary, screenshots of Menu and Groceries live, the no-clobber regeneration demonstrated, anything deferred — **and the chore-rotation check finding from Task 6, verbatim, which has now been omitted from two consecutive reports.** If the check was never actually run, say so plainly and run it now (read-only: does the chores implementation contain any automatic assignment/rotation algorithm, vs recurrence + manual assignee?). The result is incomplete without this item.

## Out of scope

Recipes in any form, nutrition, meal suggestions/AI planning, pantry quantity tracking, barcode/shopping integrations, mobile companion, notifications, at-rest UI, any cook-assignment automation.

## Guardrails (unchanged)

Plan-first with pause for maintainer confirmation — mandatory for a phase this size, with slice boundaries and files-touched laid out. Subagent-per-slice with per-slice review and a final whole-branch review (it has caught cross-cutting bugs in every task so far; a phase spanning four layers is where it earns its keep). TDD: failing test first, always, including the generation function's no-clobber and pantry-suppression properties. amity-core stays I/O-free. Comment density ≥ 50% on production crates/**/*.rs. fmt / clippy-pedantic (0 warnings) / full test suite / density gate green before every commit. Conventional Commits + DCO (`git commit -s`; multi-line via `git commit -F`). Feature branch; merge+push only after review-clean and maintainer approval.
