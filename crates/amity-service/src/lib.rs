// lib.rs — amity-service library root.
//
// Exposes the axum application as a library so integration tests can build it
// without launching a real OS process. The `main.rs` binary calls `build_app`
// with a real config; tests call it with an in-memory database.
//
// Modules:
//   config  — configuration loading from TOML
//   api     — HTTP handler modules (one per entity)

// `api` contains one sub-module per entity (inbox, task, …).
pub mod api;
pub mod config;
// `feeds` is Amity's outbound HTTP egress — currently just the ICS feed
// fetch consumed by `jobs::calendar_sync` (see feeds.rs for the egress
// guards: timeout, redirect limit, size cap).
pub mod feeds;
// `jobs` contains background maintenance tasks spawned from `main`.
pub mod jobs;

use axum::Router;
use axum::routing::{get, patch, post};
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

/// Shared state injected into every axum handler via `State<AppState>`.
///
/// Using a single state struct means handlers have a typed handle on everything
/// they need without threading individual arguments through every layer.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The database connection pool. Cloning the pool is cheap — it wraps an
    /// `Arc` internally and shares the underlying connections.
    pub db: SqlitePool,
}

/// Build the axum `Router` with all routes wired up and state injected.
///
/// Extracting this into a function (rather than building inline in `main`)
/// allows integration tests to call it with a test database without spawning a
/// real network socket.
///
/// The `TraceLayer` wraps every request in a tracing span, which gives
/// structured logs (method, path, status, latency) with zero handler boilerplate.
pub fn build_app(db: SqlitePool) -> Router {
    // Wrap the pool in the shared state struct before injecting it via `with_state`.
    let state = AppState { db };

    Router::new()
        // Routes are grouped by entity; each handler documents its own contract.
        // Inbox endpoints — see api/inbox.rs for handler documentation.
        .route("/api/v1/inbox", post(api::inbox::capture_inbox_item))
        .route("/api/v1/inbox/recent", get(api::inbox::list_recent))
        // Task endpoints — see api/task.rs for handler documentation.
        // NOTE: /upcoming must be registered before /{id} so axum does not try
        // to parse the literal "upcoming" as a UUID path segment.
        .route("/api/v1/tasks", post(api::task::create_task))
        .route("/api/v1/tasks", get(api::task::list_tasks_handler))
        // Upcoming registered before /{id} to prevent "upcoming" being parsed as a UUID.
        .route("/api/v1/tasks/upcoming", get(api::task::list_upcoming))
        .route("/api/v1/tasks/{id}", get(api::task::get_task))
        .route("/api/v1/tasks/{id}", patch(api::task::patch_task))
        // Task action endpoints — each is a POST to a named sub-resource.
        .route(
            "/api/v1/tasks/{id}/complete",
            post(api::task::complete_task),
        )
        // Skip an instance (a first-class completion event, not a deletion).
        .route("/api/v1/tasks/{id}/skip", post(api::task::skip_task))
        .route(
            "/api/v1/tasks/{id}/assignee",
            post(api::task::change_assignee),
        )
        // History is read-only (GET) — the append-only log is never modified.
        .route(
            "/api/v1/tasks/{id}/history",
            get(api::task::get_task_history),
        )
        // Event endpoints — native calendar events and their instance overrides.
        // Create and list share a path, split by method.
        .route("/api/v1/events", post(api::event::create_event))
        .route("/api/v1/events", get(api::event::list_events_handler))
        // Fetch a single event by id.
        .route("/api/v1/events/{id}", get(api::event::get_event))
        // Overlay a cancel/reschedule/annotate on one instance of an event.
        .route(
            "/api/v1/events/{id}/override",
            post(api::event::create_override),
        )
        // Surfacing — the one ranked "what's on today" query feeding the Today
        // view, drawing tasks and events into a single mixed-type list.
        .route("/api/v1/surfacing/today", get(api::surfacing::today))
        // Calendar endpoints — subscribed read-only external ICS feeds (Task 5).
        // Create and list share a path, split by method, same as events above.
        // Subscribe: builds + validates a Calendar, inserts it with fresh
        // (never-synced) sync state; see api/calendar.rs::create_calendar.
        .route("/api/v1/calendars", post(api::calendar::create_calendar))
        // List every subscribed calendar, wrapped in a `{ calendars: [...] }`
        // envelope (unlike the bare-array events list, to leave room for
        // future pagination metadata).
        .route(
            "/api/v1/calendars",
            get(api::calendar::list_calendars_handler),
        )
        // Fetch a single calendar (with its sync state) by id.
        .route("/api/v1/calendars/{id}", get(api::calendar::get_calendar))
        // Toggle whether the sync job fetches this feed; the only mutable
        // field exposed here is `enabled` (see PatchCalendarRequest).
        .route(
            "/api/v1/calendars/{id}",
            patch(api::calendar::patch_calendar),
        )
        // Unsubscribe; cascades to the calendar's own events/instances.
        // `axum::routing::delete` is used fully-qualified since only
        // `get`/`patch`/`post` are imported by name above.
        .route(
            "/api/v1/calendars/{id}",
            axum::routing::delete(api::calendar::delete_calendar_handler),
        )
        // On-demand sync of one feed: calls jobs::calendar_sync::sync_one with
        // the real feeds::fetch, bypassing the 6-hourly background job's wait.
        .route(
            "/api/v1/calendars/{id}/refresh",
            post(api::calendar::refresh_calendar),
        )
        // Attach tracing middleware so every request is logged automatically.
        // `TraceLayer` produces structured spans (method, path, status, latency)
        // for every route registered above, calendars included.
        .layer(TraceLayer::new_for_http())
        // Inject shared state into all handlers via axum's `State<AppState>` extractor.
        // Every handler that declares `State(state): State<AppState>` receives a clone.
        .with_state(state)
}
