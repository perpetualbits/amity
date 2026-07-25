// event_api.rs — integration tests for the Event API and its surfacing.
//
// These build the full axum app over a fresh in-memory database and drive real
// HTTP requests. The point of the slice is that events reach Today, so most
// tests assert on the surfacing response after creating events.

use amity_service::build_app;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── Harness ──────────────────────────────────────────────────────────────────

/// A fresh app over an isolated in-memory database. Cloning the returned Router
/// shares the same pool, so a create and a later query see each other's writes.
async fn build_test_app() -> axum::Router {
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should open");
    build_app(db)
}

/// POST a JSON body.
async fn post_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

/// GET a path.
async fn get(app: axum::Router, path: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

/// Read a response body as JSON.
async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body readable");
    serde_json::from_slice(&bytes).expect("valid JSON")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_event_then_it_surfaces_on_today() {
    let app = build_test_app().await;

    // Create a one-shot native event on a far-future day (never overdue here).
    let create = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "school play", "start_at": "2099-01-01T19:00:00+00:00" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);

    // Surface that day: the event appears as an Event-kind item.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-01-01").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["has_surfaced"], true);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "event");
    assert_eq!(items[0]["title"], "school play");
    // Events are never flagged overdue.
    assert_eq!(items[0]["overdue"], false);
    assert_eq!(items[0]["all_day"], false);
}

#[tokio::test]
async fn cancel_override_removes_the_event_from_today() {
    let app = build_test_app().await;

    // Create an event and capture its id.
    let create = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "bin day", "start_at": "2099-04-27T07:00:00+00:00" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    let id = created["id"].as_str().expect("event id");

    // Cancel it on that date.
    let cancel = post_json(
        app.clone(),
        &format!("/api/v1/events/{id}/override"),
        json!({ "instance_date": "2099-04-27", "action": "cancel" }),
    )
    .await;
    assert_eq!(cancel.status(), StatusCode::CREATED);

    // The day is now empty — the cancelled instance does not surface.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-04-27").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["has_surfaced"], false);
    assert!(body["items"].as_array().expect("items array").is_empty());
}

#[tokio::test]
async fn all_day_event_leads_the_day_ahead_of_a_task() {
    let app = build_test_app().await;

    // An all-day event and a task both due the same far-future day.
    let event = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "king's day", "start_at": "2099-04-27T00:00:00+00:00", "all_day": true }),
    )
    .await;
    assert_eq!(event.status(), StatusCode::CREATED);
    let task = post_json(
        app.clone(),
        "/api/v1/tasks",
        json!({ "title": "water plants", "due_by": "2099-04-27T15:00:00+00:00" }),
    )
    .await;
    assert_eq!(task.status(), StatusCode::CREATED);

    // The all-day event leads the mixed day.
    let resp = get(app, "/api/v1/surfacing/today?date=2099-04-27").await;
    let body = body_json(resp).await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], "event");
    assert_eq!(items[0]["all_day"], true);
    assert_eq!(items[0]["title"], "king's day");
}

#[tokio::test]
async fn create_event_validates_input() {
    let app = build_test_app().await;

    // Empty title → 422.
    let blank = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "   ", "start_at": "2099-01-01T10:00:00+00:00" }),
    )
    .await;
    assert_eq!(blank.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Incomplete recurrence pair → 422.
    let half = post_json(
        app.clone(),
        "/api/v1/events",
        json!({
            "title": "standup",
            "start_at": "2099-01-01T10:00:00+00:00",
            "recurrence_rrule": "FREQ=DAILY"
        }),
    )
    .await;
    assert_eq!(half.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A bad start datetime → 422.
    let bad_start = post_json(
        app,
        "/api/v1/events",
        json!({ "title": "x", "start_at": "not-a-date" }),
    )
    .await;
    assert_eq!(bad_start.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_event_found_and_not_found() {
    let app = build_test_app().await;

    // Create one, then fetch it back.
    let create = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "dentist", "start_at": "2099-02-02T09:00:00+00:00" }),
    )
    .await;
    let id = body_json(create).await["id"].as_str().unwrap().to_owned();
    let found = get(app.clone(), &format!("/api/v1/events/{id}")).await;
    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(body_json(found).await["title"], "dentist");

    // A valid-but-unknown id → 404.
    let missing = get(
        app.clone(),
        "/api/v1/events/018f1a2b-0000-7000-8000-000000000099",
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // A malformed id → 400.
    let bad = get(app, "/api/v1/events/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn override_on_missing_event_is_404() {
    let app = build_test_app().await;
    // No such event → 404.
    let resp = post_json(
        app,
        "/api/v1/events/018f1a2b-0000-7000-8000-000000000099/override",
        json!({ "instance_date": "2099-01-01", "action": "cancel" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
