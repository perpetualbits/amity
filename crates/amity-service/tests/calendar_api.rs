// calendar_api.rs — integration tests for the Calendars API and its
// end-to-end surfacing.
//
// These build the full axum app over a fresh in-memory database and drive
// real HTTP requests, mirroring event_api.rs. Covers the CRUD surface (create
// /list/get/patch/delete, plus id-validation and builder-error status codes)
// and — the important one — that an event ingested by the sync job (with an
// injected fetch, so no real network is touched) surfaces on Today exactly
// like a native event does.

use amity_service::build_app;
use amity_service::feeds::FetchError;
use amity_service::jobs::calendar_sync::run_once;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::future::ready;
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

/// PATCH a JSON body.
async fn patch_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    let request = Request::builder()
        .method("PATCH")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

/// DELETE a path (no body).
async fn delete(app: axum::Router, path: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("DELETE")
        .uri(path)
        .body(Body::empty())
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
async fn create_lists_and_deletes_a_calendar() {
    let app = build_test_app().await;

    // Create — 201 with the row, never-synced by default.
    let create = post_json(
        app.clone(),
        "/api/v1/calendars",
        json!({ "name": "holidays", "url": "https://example.test/h.ics", "category": "holiday" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert_eq!(created["name"], "holidays");
    assert_eq!(created["category"], "holiday");
    assert_eq!(created["enabled"], true);
    assert_eq!(created["last_status"], "never");
    assert_eq!(created["event_count"], 0);
    let id = created["id"].as_str().unwrap().to_owned();

    // List — the one calendar we just created.
    let list = get(app.clone(), "/api/v1/calendars").await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = body_json(list).await;
    let calendars = list_body["calendars"].as_array().unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0]["id"], id);

    // Get by id.
    let got = get(app.clone(), &format!("/api/v1/calendars/{id}")).await;
    assert_eq!(got.status(), StatusCode::OK);
    assert_eq!(body_json(got).await["name"], "holidays");

    // Delete — 200, then a subsequent GET is 404.
    let del = delete(app.clone(), &format!("/api/v1/calendars/{id}")).await;
    assert_eq!(del.status(), StatusCode::OK);
    let after = get(app.clone(), &format!("/api/v1/calendars/{id}")).await;
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn blank_name_is_422_and_bad_scheme_is_422() {
    let app = build_test_app().await;

    // Blank name (whitespace-only) fails the builder's EmptyName check.
    let blank = post_json(
        app.clone(),
        "/api/v1/calendars",
        json!({ "name": "  ", "url": "https://example.test/x.ics" }),
    )
    .await;
    assert_eq!(blank.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A non-http(s) scheme fails the builder's URL normalisation.
    let scheme = post_json(
        app,
        "/api/v1/calendars",
        json!({ "name": "bad", "url": "file:///etc/passwd" }),
    )
    .await;
    assert_eq!(scheme.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn patch_toggles_enabled() {
    let app = build_test_app().await;

    let create = post_json(
        app.clone(),
        "/api/v1/calendars",
        json!({ "name": "club", "url": "https://example.test/c.ics" }),
    )
    .await;
    let id = body_json(create).await["id"].as_str().unwrap().to_owned();

    // Disable it.
    let patch = patch_json(
        app.clone(),
        &format!("/api/v1/calendars/{id}"),
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(patch.status(), StatusCode::OK);
    assert_eq!(body_json(patch).await["enabled"], false);

    // The disabled flag persists across a fresh read.
    let got = get(app.clone(), &format!("/api/v1/calendars/{id}")).await;
    assert_eq!(body_json(got).await["enabled"], false);

    // A patch against an unknown id is a 404.
    let missing = patch_json(
        app,
        "/api/v1/calendars/018f1a2b-0000-7000-8000-000000000099",
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_id_is_400() {
    let app = build_test_app().await;

    let bad_get = get(app.clone(), "/api/v1/calendars/not-a-uuid").await;
    assert_eq!(bad_get.status(), StatusCode::BAD_REQUEST);

    let bad_patch = patch_json(
        app.clone(),
        "/api/v1/calendars/not-a-uuid",
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(bad_patch.status(), StatusCode::BAD_REQUEST);

    let bad_delete = delete(app, "/api/v1/calendars/not-a-uuid").await;
    assert_eq!(bad_delete.status(), StatusCode::BAD_REQUEST);
}

/// A two-event ICS fixture: one event on 2099-05-01 (the day we query), one on
/// 2099-05-02 (a control day so we can tell surfacing is date-filtering, not
/// just dumping everything).
const FIXTURE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:school-play\r\nSUMMARY:school play\r\nDTSTART:20990501T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:other-day\r\nSUMMARY:other day\r\nDTSTART:20990502T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[tokio::test]
async fn ingested_external_event_surfaces_on_today() {
    // Build the pool ourselves (rather than via `build_test_app`) so we can
    // hand the *same* pool to both the HTTP app and `run_once` below — the
    // ingested rows must land in the database the surfacing request reads.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should open");
    let app = build_app(db.clone());

    // Subscribe a calendar via the real API — this is what the household does.
    let create = post_json(
        app.clone(),
        "/api/v1/calendars",
        json!({ "name": "school", "url": "https://example.test/school.ics", "category": "school" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);

    // Drive ingestion through the sync job directly with an injected fetch
    // closure returning fixture ICS text — the real `refresh` handler would
    // hit the network, so the e2e must not exercise it (mirrors Task 4's
    // tests/calendar_sync.rs). `run_once` loads every enabled calendar itself
    // (the one just created, above) and ingests its events.
    let now = time::macros::datetime!(2099-04-27 12:00:00 UTC);
    let report = run_once(&db, now, |_url| {
        ready(Ok::<_, FetchError>(FIXTURE_ICS.to_owned()))
    })
    .await
    .expect("sync run succeeds");
    assert_eq!(report.calendars_synced, 1);
    assert_eq!(report.events_upserted, 2);

    // The event landing on the queried day surfaces; the control event on the
    // following day does not — proving surfacing is genuinely date-filtering
    // ingested (not just native) events.
    let resp = get(app.clone(), "/api/v1/surfacing/today?date=2099-05-01").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["has_surfaced"], true);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "event");
    assert_eq!(items[0]["title"], "school play");

    let other_day = get(app, "/api/v1/surfacing/today?date=2099-05-02").await;
    let other_body = body_json(other_day).await;
    let other_items = other_body["items"].as_array().expect("items array");
    assert_eq!(other_items.len(), 1);
    assert_eq!(other_items[0]["title"], "other day");
}
