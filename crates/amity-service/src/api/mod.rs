// api/mod.rs — HTTP API module.
//
// This module owns the axum router and all handler modules.
// Each entity gets its own handler module; the router here wires them together.
//
// All routes are under `/api/v1/` so that a future breaking change can be
// deployed alongside the current version without path conflicts.
//
// Current routes:
//   POST  /api/v1/inbox                    — capture a new inbox item
//   GET   /api/v1/inbox/recent             — list recent inbox items
//   POST  /api/v1/tasks                    — create a task
//   GET   /api/v1/tasks                    — list tasks (with filters)
//   GET   /api/v1/tasks/upcoming           — upcoming recurring instances
//   GET   /api/v1/tasks/{id}              — fetch a task
//   PATCH /api/v1/tasks/{id}             — update a task
//   POST  /api/v1/tasks/{id}/complete    — mark done
//   POST  /api/v1/tasks/{id}/skip        — mark skipped
//   POST  /api/v1/tasks/{id}/assignee    — change current_assignee_id
//   GET   /api/v1/tasks/{id}/history     — list completion log entries
//   GET   /api/v1/surfacing/today        — ranked "what's on today" query
//   GET   /api/v1/week                   — Monday-start 7-day layout query
//   POST  /api/v1/calendars              — subscribe to a new ICS feed
//   GET   /api/v1/calendars              — list subscribed calendars
//   GET   /api/v1/calendars/{id}         — fetch one calendar (+ sync state)
//   PATCH /api/v1/calendars/{id}         — enable/disable a subscription
//   DELETE /api/v1/calendars/{id}        — unsubscribe (cascades its events)
//   POST  /api/v1/calendars/{id}/refresh — sync one feed on demand
//   POST  /api/v1/meals                             — plan a new meal
//   GET   /api/v1/meals                             — list meals (?from=&to=)
//   GET   /api/v1/meals/{id}                        — fetch a meal
//   DELETE /api/v1/meals/{id}                       — remove a meal
//   POST  /api/v1/grocery-lists                     — create a grocery list
//   GET   /api/v1/grocery-lists                     — list grocery lists
//   GET   /api/v1/grocery-lists/{id}                — fetch a grocery list
//   POST  /api/v1/grocery-lists/{id}/items          — add an item manually
//   GET   /api/v1/grocery-lists/{id}/items          — list a list's items
//   POST  /api/v1/grocery-lists/{id}/generate       — generate additions from
//                                                      planned meals
//   PATCH /api/v1/grocery-items/{id}                — toggle checked
//   DELETE /api/v1/grocery-items/{id}               — remove an item
//   POST  /api/v1/pantry                            — record a new staple
//   GET   /api/v1/pantry                            — list pantry staples
//   DELETE /api/v1/pantry/{id}                      — remove a staple

pub mod calendar;
pub mod event;
pub mod grocery;
pub mod inbox;
pub mod meal;
pub mod pantry;
pub mod surfacing;
pub mod task;
