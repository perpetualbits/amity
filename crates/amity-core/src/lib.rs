// lib.rs — amity-core public API.
//
// amity-core holds all domain types and business logic with no I/O dependencies.
// Nothing in this crate touches the filesystem, network, or a database — that
// separation makes every module here testable without infrastructure.
//
// Crate dependency order (all arrows go downward; no upward dependencies):
//   amity-service → amity-storage → amity-core
//
// Modules:
//   ids                     — typed ID newtypes for all entities
//   ics                     — pure ICS (RFC 5545) parsing and external-recurrence expansion
//   inbox                   — InboxItem domain type and its builder
//   recurrence              — RecurrenceRule type and RRULE validation
//   recurrence_materialiser — RFC 5545 instance generation (uses rrule crate)
//   task                    — Task domain type, builder, enums
//   completion_log          — CompletionLog domain type

/// Typed ID newtypes. See module docs for the rationale.
pub mod ids;

/// `ParsedEvent`, `IcsError`, `parse_feed`, and `expand_external`.
///
/// Pure iCalendar (RFC 5545) parsing and recurrence expansion for read-only
/// external feeds. No I/O — the sync job fetches bytes; this module only
/// parses the string it is handed. See brief §7 (calendars & time).
pub mod ics;

/// `InboxItem`, `InboxSource`, `TriageState`, `TypedEntityRef`, and `InboxItemBuilder`.
///
/// The Inbox is the first promise Amity makes — see brief §6.3.
pub mod inbox;

/// `RecurrenceRule` and RRULE validation against the supported subset.
///
/// The supported subset is DAILY/WEEKLY/MONTHLY/YEARLY with BYDAY, BYMONTHDAY,
/// INTERVAL, COUNT, and UNTIL. See `docs/adrs/0002-recurrence-engine.md`.
pub mod recurrence;

/// Instance materialisation: `RecurrenceRule` → `Vec<OffsetDateTime>`.
///
/// Delegates RFC 5545 parsing and DST handling to the `rrule` crate.
/// Converts outputs back to `time::OffsetDateTime` at the boundary.
pub mod recurrence_materialiser;

/// `Task`, `TaskBuilder`, `TaskStatus`, `EffortLevel`, `Priority`, `TaskError`.
///
/// Task is the most-used entity in Amity — see brief §6.5 and §8.
pub mod task;

/// `Calendar`, `CalendarBuilder`, `CalendarCategory`, `SyncStatus`, `CalendarError`.
///
/// One subscribed external ICS feed. Amity is a calendar aggregator — the
/// household subscribes to read-only feeds (school, waste, holidays, personal)
/// and the hub displays their events. See brief §7 and
/// `docs/superpowers/specs/2026-07-26-task-5-ics-ingestion-design.md`.
pub mod calendar;

/// `CompletionLog` — the immutable record of a Task instance being completed.
///
/// The substrate for the household's own fairness conversations.
/// See brief §6.5 (`CompletionLog`) and §8.2 (chore-completion model).
pub mod completion_log;

/// `SurfacedItem`, `SurfaceCandidate`, `SurfacingConfig`, and `rank_today`.
///
/// The ranked "what's on today" query as pure domain logic — where Amity
/// chooses information over pressure. See brief §6.4 (surfacing) and §3
/// (the designed empty state), and `docs/task_3_surfacing_and_today_view.md`.
pub mod surfacing;

/// `Event`, `EventSource`, `EventSourceKind`, `EventBuilder`, `EventError`.
///
/// Calendar events — native and read-only external. Amity is an aggregator,
/// not a source of truth. See brief §6.5 (Event) and §7 (calendars & time),
/// and `docs/task_4_event_and_calendar.md`.
pub mod event;

/// `EventOverride` — a local overlay on a read-only external event instance.
///
/// Lets the household record "bin day moved for King's Day" without writing
/// back to the source. See brief §6.5 (`EventOverride`).
pub mod event_override;

/// `GrocerySource`, `GroceryList`, `GroceryListBuilder`, `GroceryItem`,
/// `GroceryItemBuilder`, `GroceryError`, and `plan_grocery_additions`.
///
/// P2 Slice 1 (meals, lists & pantry): the pure grocery-generation function
/// that turns planned meals into grocery-list additions without clobbering
/// pantry staples or items already on the list.
pub mod grocery;

/// `MealSlot`, `IngredientLine`, `Meal`, `MealBuilder`, `MealError`.
///
/// P2 Slice 1 (meals, lists & pantry): a single planned meal on a single
/// date, with freetext ingredient lines. No recipes or cook-assignment logic.
pub mod meal;

/// `Member`, `MemberBuilder`, `MemberColor`, `MemberError`.
///
/// Task 9 Slice 1: a household member as a display-registry entry only —
/// name, optional initial, optional colour. No accounts, roles, activity, or
/// age/child-specific structure. See the module doc for the full boundary.
pub mod member;

/// `PantryItem`, `PantryItemBuilder`, `PantryError`.
///
/// P2 Slice 1 (meals, lists & pantry): a lightweight staples-memory used to
/// suppress already-stocked ingredients during grocery generation. No
/// levels, thresholds, or depletion tracking.
pub mod pantry;
