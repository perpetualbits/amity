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

// The axum application type this module builds and returns.
use axum::Router;
// The three HTTP methods every route below dispatches on (DELETE is used
// fully-qualified per-call instead, since it is not needed by name elsewhere).
use axum::routing::{get, patch, post};
// The shared connection pool type stored on `AppState`.
use sqlx::SqlitePool;
// Structured per-request tracing spans, attached once via `.layer(...)` below.
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
    // P2 Slice 3 adds three route groups below the calendar block: meals,
    // grocery lists/items, and pantry. Each module documents its own
    // endpoints and error mapping — see api/meal.rs, api/grocery.rs, and
    // api/pantry.rs — so the comments here stay short (path + method only).
    // Wrap the pool in the shared state struct before injecting it via `with_state`.
    let state = AppState { db };

    Router::new()
        // Routes are grouped by entity; each handler documents its own contract.
        // Inbox endpoints — see api/inbox.rs for handler documentation.
        // Capture: the household's universal drop point for anything unsorted.
        .route("/api/v1/inbox", post(api::inbox::capture_inbox_item))
        // Read-only: the hub's own inbox triage list.
        .route("/api/v1/inbox/recent", get(api::inbox::list_recent))
        // Task endpoints — see api/task.rs for handler documentation.
        // NOTE: /upcoming must be registered before /{id} so axum does not try
        // to parse the literal "upcoming" as a UUID path segment.
        .route("/api/v1/tasks", post(api::task::create_task))
        // Every task, filterable via query params (see api/task.rs).
        .route("/api/v1/tasks", get(api::task::list_tasks_handler))
        // Upcoming registered before /{id} to prevent "upcoming" being parsed as a UUID.
        .route("/api/v1/tasks/upcoming", get(api::task::list_upcoming))
        // Fetch a single task by id.
        .route("/api/v1/tasks/{id}", get(api::task::get_task))
        // Partial update of a task's mutable fields.
        .route("/api/v1/tasks/{id}", patch(api::task::patch_task))
        // Task action endpoints — each is a POST to a named sub-resource.
        // Mark an instance done; writes an append-only CompletionLog entry.
        .route(
            "/api/v1/tasks/{id}/complete",
            post(api::task::complete_task),
        )
        // Skip an instance (a first-class completion event, not a deletion).
        .route("/api/v1/tasks/{id}/skip", post(api::task::skip_task))
        // Reassign the current responsible member.
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
        // Every native and ingested event, by start time.
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
        // Week — the Monday-start 7-day layout query, mirroring Today's shape
        // and error handling one level up (a week of days, not a day of items).
        .route("/api/v1/week", get(api::surfacing::week))
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
        // Meal endpoints — planned meals, feeding both grocery generation and
        // Today's meal surfacing. No PATCH here (see api/meal.rs's doc
        // comment for why editing is deferred).
        // Create and list share a path, split by method, same as events above.
        // Plan a new meal (name, date, slot, cook, ingredients, notes).
        .route("/api/v1/meals", post(api::meal::create_meal))
        // List supports ?from=&to= filtering; see api/meal.rs.
        .route("/api/v1/meals", get(api::meal::list_meals_handler))
        // Fetch a single meal (with its ordered ingredient lines) by id.
        .route("/api/v1/meals/{id}", get(api::meal::get_meal))
        // Remove a meal; cascades to its meal_ingredients rows.
        .route(
            "/api/v1/meals/{id}",
            axum::routing::delete(api::meal::delete_meal_handler),
        )
        // Grocery list endpoints — create/list/fetch a list, manage its
        // items, and generate additions from planned meals (this phase's
        // payoff — see api/grocery.rs's module doc for the endpoint's
        // contract).
        // Create a new (empty) list.
        .route(
            "/api/v1/grocery-lists",
            post(api::grocery::create_grocery_list),
        )
        // List every grocery list.
        .route(
            "/api/v1/grocery-lists",
            get(api::grocery::list_grocery_lists_handler),
        )
        // Fetch a single list by id.
        .route(
            "/api/v1/grocery-lists/{id}",
            get(api::grocery::get_grocery_list),
        )
        // Add an item manually (source: "manual").
        .route(
            "/api/v1/grocery-lists/{id}/items",
            post(api::grocery::create_grocery_item),
        )
        // List a list's items.
        .route(
            "/api/v1/grocery-lists/{id}/items",
            get(api::grocery::list_grocery_items_handler),
        )
        // Generate additions from planned meals in a date range (default:
        // this week) — see api/grocery.rs::generate_grocery_items.
        .route(
            "/api/v1/grocery-lists/{id}/generate",
            post(api::grocery::generate_grocery_items),
        )
        // Grocery item endpoints — a flat `/grocery-items/{id}` namespace
        // (not nested under a list) since these two actions only need the
        // item's own id.
        // Toggle checked; the only mutable field on an item.
        .route(
            "/api/v1/grocery-items/{id}",
            patch(api::grocery::patch_grocery_item),
        )
        // Remove an item.
        .route(
            "/api/v1/grocery-items/{id}",
            axum::routing::delete(api::grocery::delete_grocery_item_handler),
        )
        // Pantry endpoints — the household's staples memory that grocery
        // generation consults to skip already-stocked ingredients.
        // Record a new staple (name + optional note).
        .route("/api/v1/pantry", post(api::pantry::create_pantry_item))
        // List every staple; the pantry list is small, so no filters exist.
        .route(
            "/api/v1/pantry",
            get(api::pantry::list_pantry_items_handler),
        )
        // Remove a staple; there is no update path (see api/pantry.rs).
        .route(
            "/api/v1/pantry/{id}",
            axum::routing::delete(api::pantry::delete_pantry_item_handler),
        )
        // Attach tracing middleware so every request is logged automatically.
        // `TraceLayer` produces structured spans (method, path, status, latency)
        // for every route registered above, meals/groceries/pantry included.
        .layer(TraceLayer::new_for_http())
        // Inject shared state into all handlers via axum's `State<AppState>` extractor.
        // Every handler that declares `State(state): State<AppState>` receives a clone.
        // Cloning `AppState` only clones the pool's cheap internal `Arc`, not
        // a connection, so every handler shares the same underlying pool.
        .with_state(state)
}
