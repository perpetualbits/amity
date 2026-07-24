// surfacing_api.rs — integration tests for the surfacing (Today) HTTP API.
//
// Each test builds the full axum application (production code, not a mock)
// backed by an in-memory SQLite database and sends real HTTP requests via
// `tower::ServiceExt::oneshot`. The pure ranking rule is unit-tested in
// `amity_core::surfacing`; these tests verify the endpoint that assembles
// candidates from storage, ranks them, and serialises the result.
//
// Determinism: the handler compares against the real wall-clock `now`, so the
// tests pin the queried day far in the future (2099) where possible. A task due
// in 2099 is never "overdue" relative to a 2026 clock, so window-based cases are
// stable regardless of when the suite runs. The recurring case instead discovers
// a real materialised instance date via `/tasks/upcoming` and queries that.
//
// What these tests verify:
//   • empty state: a day with nothing due returns has_surfaced=false, items=[].
//   • one-shot due that day surfaces as a "task" item, not overdue.
//   • a task whose window has not opened yet stays off the day.
//   • a recurring instance surfaces on its own scheduled day.
//
// Everything goes through the production router (build_app) and the real
// storage layer — there are no mocks, so a passing test exercises the same code
// path the hub will hit in production.

// The production `build_app` wires all routes (including surfacing) with a pool.
use amity_service::build_app;
// `open_database` applies all migrations before returning the pool.
use amity_storage::connection::open_database;
// `Body` wraps raw bytes for HTTP request bodies.
use axum::body::Body;
// `Request` is the HTTP request type; `StatusCode` for status assertions.
use axum::http::{Request, StatusCode};
// `json!` builds JSON request bodies; `Value` for flexible response assertions.
use serde_json::{Value, json};
// `ServiceExt` provides the `oneshot` method that drives one request in-process.
use tower::ServiceExt;

// ─── Test helpers ─────────────────────────────────────────────────────────────

/// Build a test application backed by a fresh in-memory database.
///
/// Each call creates an isolated `:memory:` database, so parallel tests never
/// see each other's writes. `build_app` is the production router — no fork.
async fn build_test_app() -> axum::Router {
    // A new, empty SQLite database on every call keeps tests isolated.
    // `open_database` also applies all migrations before returning the pool.
    let db = open_database("sqlite::memory:")
        .await
        // Opening an in-memory database cannot fail under normal conditions.
        .expect("in-memory database should always open");
    // Wire the production routes onto the fresh pool and return the router.
    build_app(db)
}

/// Issue a `POST` with a JSON body and return the response.
///
/// Sets `Content-Type: application/json` so axum's `Json` extractor accepts it.
async fn post_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    // Build the POST request with the JSON body and required Content-Type.
    let request = Request::builder()
        // The HTTP method; axum matches the exact uppercase string.
        .method("POST")
        // The target path supplied by the caller.
        .uri(path)
        // Without this header axum returns 415 Unsupported Media Type.
        .header("Content-Type", "application/json")
        // Serialise the JSON value to bytes for the body.
        .body(Body::from(body.to_string()))
        // A builder error is a test bug, not a runtime condition.
        .expect("request builder should not fail");
    // Drive the request through the service without opening a socket.
    app.oneshot(request)
        // Await the in-process response.
        .await
        // A tower service error would indicate a routing/middleware bug.
        .expect("service call should not fail")
}

/// Issue a `GET` and return the response. GET requests carry no body.
async fn get(app: axum::Router, path: &str) -> axum::response::Response {
    // Build the GET request with an empty body.
    let request = Request::builder()
        // GET is read-only and side-effect free.
        .method("GET")
        // The target path (may include a query string).
        .uri(path)
        // `Body::empty()` is the zero-cost placeholder for a bodyless request.
        .body(Body::empty())
        // A builder error is a test bug, not a runtime condition.
        .expect("request builder should not fail");
    // Drive the GET request in-process.
    app.oneshot(request)
        // Await the in-process response.
        .await
        // A tower service error would indicate a routing/middleware bug.
        .expect("service call should not fail")
}

/// Deserialise the response body as a `serde_json::Value`.
async fn body_json(response: axum::response::Response) -> Value {
    // Read the whole body; `usize::MAX` avoids truncating larger payloads.
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    // Parse as JSON; a failure means the handler returned a non-JSON body.
    serde_json::from_slice(&bytes).expect("response body should be valid JSON")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn surfacing_today_is_empty_when_nothing_is_due() {
    // A day with no tasks must return the designed empty state, not an error.
    // The `has_surfaced` flag is what the hub keys the calm "nothing today" off,
    // so it must be present and false even when there is genuinely nothing to do.
    // A fresh app over an empty in-memory database.
    let app = build_test_app().await;
    // Query a far-future day on a fresh database — nothing can be due.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-01-01").await;
    // The endpoint must exist and answer 200 (this is what fails first, RED).
    assert_eq!(resp.status(), StatusCode::OK);
    // Inspect the JSON envelope.
    let body = body_json(resp).await;
    // The queried date is echoed back so the client can label the view.
    assert_eq!(body["date"], "2099-01-01");
    // Nothing surfaced on this day.
    assert_eq!(body["has_surfaced"], false);
    // The items array is present but empty — the calm "nothing today" state.
    let items = body["items"].as_array().expect("items must be an array");
    // No items on a day with no tasks.
    assert!(items.is_empty());
}

#[tokio::test]
async fn surfacing_today_includes_one_shot_task_due_that_day() {
    // A one-shot task whose deadline falls on the queried day must surface as an
    // ordinary (not overdue) item — the everyday case the Today view is built for.
    // A fresh app over an empty in-memory database.
    let app = build_test_app().await;

    // Create a one-shot task due on 2099-01-01 (far future → never overdue here).
    // The task is captured through the real POST /tasks handler, not seeded directly.
    let create = post_json(
        app.clone(),
        "/api/v1/tasks",
        json!({
            // A plain, recognisable subject.
            "title": "call the dentist",
            // Due at 09:00 UTC on the queried day.
            "due_by": "2099-01-01T09:00:00+00:00"
        }),
    )
    // Send the request and await the in-process response.
    .await;
    // Creation must succeed before we can surface it.
    assert_eq!(create.status(), StatusCode::CREATED);

    // Ask for that exact day.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-01-01").await;
    // The endpoint answers 200.
    assert_eq!(resp.status(), StatusCode::OK);
    // Read the envelope.
    let body = body_json(resp).await;
    // Something surfaced on this day.
    assert_eq!(body["has_surfaced"], true);
    // Grab the items array — exactly the one task we created should be present.
    let items = body["items"].as_array().expect("items must be an array");
    // No other tasks exist in this fresh database.
    assert_eq!(items.len(), 1);
    // The item is Task-kind (the only kind today).
    assert_eq!(items[0]["kind"], "task");
    // Its title round-tripped through creation and surfacing unchanged.
    assert_eq!(items[0]["title"], "call the dentist");
    // It is not overdue, because the deadline is far in the future.
    assert_eq!(items[0]["overdue"], false);
}

#[tokio::test]
async fn surfacing_today_excludes_task_before_its_window_opens() {
    // A task not actionable until March must not appear on a January day: the
    // rule shows a windowed task only once its window has opened, avoiding the
    // premature nagging that would erode trust in the surface.
    // A fresh app over an empty in-memory database.
    let app = build_test_app().await;

    // Create a one-shot task whose window opens on 2099-03-01 and closes 2099-06-01.
    let create = post_json(
        app.clone(),
        "/api/v1/tasks",
        json!({
            // A task that is deliberately not yet actionable in January.
            "title": "book summer camp",
            // Not meaningful to start before March.
            "earliest_at": "2099-03-01T00:00:00+00:00",
            // Due by June.
            "due_by": "2099-06-01T00:00:00+00:00"
        }),
    )
    // Send the request and await the in-process response.
    .await;
    // Creation must succeed.
    assert_eq!(create.status(), StatusCode::CREATED);

    // Query a day before the window opens.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-01-01").await;
    // The endpoint answers 200.
    assert_eq!(resp.status(), StatusCode::OK);
    // Read the envelope.
    let body = body_json(resp).await;
    // Premature nagging is avoided: the day stays empty.
    assert_eq!(body["has_surfaced"], false);
    // The items array is present but empty.
    let items = body["items"].as_array().expect("items must be an array");
    // A task whose window has not opened makes no claim on today.
    assert!(items.is_empty());
}

#[tokio::test]
async fn surfacing_today_includes_recurring_instance_on_its_scheduled_day() {
    // A recurring task's materialised instance must surface on its own day.
    // Rather than compute the instance date (timezone-sensitive), we discover a
    // real one via /tasks/upcoming and then query surfacing for that exact date.
    //
    // All three routers share one pool so each sees the others' writes; a fresh
    // `:memory:` database per test keeps them isolated from other tests.
    let db = open_database("sqlite::memory:")
        // Await the pool (migrations applied inside).
        .await
        // Opening an in-memory database cannot fail under normal conditions.
        .expect("in-memory db");
    // Router that creates the recurring task.
    let app_create = build_app(db.clone());
    // Router that reads /upcoming to discover an instance date.
    let app_upcoming = build_app(db.clone());
    // Router that reads /surfacing/today for that date.
    let app_surfacing = build_app(db);

    // Create a daily recurring task so an instance falls within the horizon soon.
    let create = post_json(
        app_create,
        "/api/v1/tasks",
        json!({
            // The recurring subject under test.
            "title": "daily standup",
            // Every day; the materialiser fills the next 60 days.
            "recurrence_rrule": "FREQ=DAILY",
            // Amsterdam timezone for DST-correct materialisation.
            "recurrence_timezone": "Europe/Amsterdam"
        }),
    )
    // Send the request and await the in-process response.
    .await;
    // Creation (and initial materialisation) must succeed.
    assert_eq!(create.status(), StatusCode::CREATED);

    // ── Discover a real instance date via the upcoming endpoint ─────────────
    // Fetch the pre-materialised instances for the recurring task.
    let upcoming = get(app_upcoming, "/api/v1/tasks/upcoming").await;
    // The upcoming endpoint answers 200.
    assert_eq!(upcoming.status(), StatusCode::OK);
    // Parse the instances list.
    let upcoming_body = body_json(upcoming).await;
    // It is a JSON array of instances.
    let instances = upcoming_body.as_array().expect("upcoming must be an array");
    // There must be at least one instance to surface.
    assert!(
        !instances.is_empty(),
        "a daily rule must materialise instances"
    );
    // The scheduled_at is RFC 3339 ("YYYY-MM-DDThh:mm:ss±hh:mm"); take the date.
    let scheduled_at = instances[0]["scheduled_at"]
        .as_str()
        .expect("scheduled_at must be a string");
    // The first ten characters are the offset-local calendar date.
    let day = &scheduled_at[..10];

    // ── Surface that exact day and expect the recurring item ────────────────
    // Query surfacing for the discovered day.
    let resp = get(
        app_surfacing,
        &format!("/api/v1/surfacing/today?date={day}"),
    )
    // Send the request and await the in-process response.
    .await;
    // The surfacing endpoint answers 200.
    assert_eq!(resp.status(), StatusCode::OK);
    // Read the envelope.
    let body = body_json(resp).await;
    // The recurring task's instance surfaces on its own day.
    assert_eq!(body["has_surfaced"], true);
    // Pull the items array.
    let items = body["items"].as_array().expect("items must be an array");
    // Our recurring title must be among the surfaced items.
    assert!(
        items.iter().any(|i| i["title"] == "daily standup"),
        "the recurring instance must surface on its scheduled day"
    );
}
