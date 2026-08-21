# Amity — Task 6b: Unblock the hub (E0255 is not upstream), run live, ship Week UI

*Brief prepared by Claude.ai, 2026-08-20. Supersedes the "tauri family downgrade" resume steps in docs/PROJECT_STATUS.md — do not downgrade anything.*

## The re-diagnosis

The deferred-hub report attributed the build failure to an upstream tauri-macros 2.6.3 bug on rustc 1.88–1.95. That diagnosis is almost certainly wrong. E0255 (`__cmd__* is defined multiple times`) is a long-standing, version-independent property of `#[tauri::command]` glue generation, documented in the current Tauri v2 docs: a command function marked `pub` in the same file that invokes `tauri::generate_handler!` collides with its own generated macro. It has presented identically since Tauri v1. It is not a rustc regression, and downgrading the tauri family cannot fix it — which is consistent with the downgrade attempt cascading instead of resolving.

The likely trigger is Task 6's own restructure of `apps/hub-tauri/src-tauri` (new `[workspace]` root), which changed where command fns sit relative to `generate_handler!`.

## Slice 1 — Fix the build (expected: small)

1. On the current toolchain and latest tauri 2.x (no version changes), reproduce the E0255 and read which `__cmd__*` names collide.
2. Apply the documented fix, whichever fits the current layout:
   - commands co-located with `generate_handler!` in `lib.rs`/`main.rs` → remove `pub` from them; **or**
   - move commands into a dedicated `commands` module (there they *must* be `pub`) and register them from `lib.rs` via `commands::name` / a `use` that does not re-export the generated `__cmd__` macros. Watch for glob re-exports (`pub use commands::*`) — they recreate the collision.
3. `cargo build` in `src-tauri` green. If, after the fix is verifiably applied (collision names gone, layout matches the documented pattern), a *different* genuinely upstream error remains: stop, capture the exact error, and report — do not improvise version changes.

## Slice 2 — Resume Task 6 as scoped

With the build green, execute the original Task 6 Slice 0 and Slice 2b, unchanged:
- Hub launches live on this laptop (`cargo tauri dev` path), Capture/Today/Week render and respond to touch; single documented launch command; prerequisites recorded.
- Week UI per the Task 6 brief: read-mostly, touch-first, today marked, prev/next/this-week navigation, overflow handling, external-source / rescheduled / annotation markers. **Ordering is now ratified: all-day → timed events → tasks, as shipped in plan_week. Do not change it.**
- Visual verification against the seeded scenarios (external recurring with EXDATE, rescheduled, annotated, all-day, dated task); screenshots in the result.

## Slice 3 — Housekeeping

- Correct docs/PROJECT_STATUS.md: replace the upstream-bug diagnosis and downgrade resume-steps with the actual cause and fix; update the "next" marker (on completion → Task 7: at-rest UI + weather, marked but not scoped).
- Include in the result the chore-rotation check finding from Task 6, which was marked done but never reported.

## Out of scope

P2 meals/lists/pantry, at-rest UI, weather, notifications, JS test runner beyond a trivial smoke test, any tauri/rustc version changes.

## Guardrails (unchanged)

Plan-first with pause for confirmation; TDD where testable (build-fix slice is exempt from test-first, the Week UI is not exempt from visual verification); fmt/clippy-pedantic/tests/comment-density all green before commit; Conventional Commits + DCO via `git commit -s`, multi-line messages via `git commit -F`; feature branch, merge+push only after review-clean and maintainer approval.
