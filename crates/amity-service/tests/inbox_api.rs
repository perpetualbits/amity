// inbox_api.rs — integration tests for the Inbox HTTP API.
//
// Each test spins up the full axum application (same code as production) with
// an in-memory SQLite database, then issues real HTTP requests via `reqwest`
// against an in-process listener. No mocking, no faking of the storage layer.
//
// Why in-process rather than a real network socket? In-process `oneshot` gives
// deterministic teardown (the pool is dropped at test end), no port conflicts
// between parallel tests, and no OS-level socket overhead. The application code
// is identical to production; only the transport changes.
//
// This approach catches:
//   • Route registration mistakes (wrong path, wrong method).
//   • Serialisation/deserialisation errors (request body, response shape).
//   • Business rule enforcement (empty text → 422, limit > 100 → 400).
//   • End-to-end data flow (POST then GET shows the created item).

use amity_service::build_app;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
// `json!` macro for building request bodies without manual string formatting.
// `Value` for asserting response structure without defining full structs.
use serde_json::{Value, json};
// `ServiceExt` provides the `oneshot` method that drives a request through the
// axum Router without binding to a port.
use tower::ServiceExt;

// ─── Test helper ─────────────────────────────────────────────────────────────

/// Build a test application backed by an in-memory database.
///
/// Returns the axum `Router` ready to accept requests via `oneshot`.
/// Each test calls this independently so there is no shared state between
/// tests — even when the test runner runs them in parallel.
///
/// The in-memory database is isolated per invocation because `:memory:` in
/// `SQLite` creates a new, empty database each time. Migrations are applied
/// automatically via `open_database`.
async fn build_test_app() -> axum::Router {
    // In-memory SQLite — isolated per test, migrations applied automatically.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should always open");
    // `build_app` is the same function used in production; this test uses the
    // exact same code path, not a separate test-only implementation.
    // Returns the ready-to-use Router.
    build_app(db)
}

/// Issue a POST request with a JSON body and return the response.
///
/// Serialises `body` to a JSON string and sets the `Content-Type` header so
/// axum's `Json` extractor recognises it as JSON.
async fn post_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    // Build the request with method, path, Content-Type header, and serialised body.
    let request = Request::builder()
        .method("POST")
        .uri(path)
        // Without this header, axum's Json extractor returns 415 Unsupported Media Type.
        .header("Content-Type", "application/json")
        // Serialise the JSON Value to a string for the request body.
        .body(Body::from(body.to_string()))
        // request builder errors are programmer errors, not runtime conditions.
        .expect("request builder should not fail");

    // `oneshot` drives a single request through the service without binding to
    // a network socket. The service is consumed, so each test call needs a
    // fresh app instance (or we share via Clone, which Router supports).
    // Drive the request through the service.
    app.oneshot(request)
        .await
        // A service-level error is a bug, not an expected outcome.
        .expect("service call should not fail")
}

/// Issue a GET request and return the response.
async fn get(app: axum::Router, path: &str) -> axum::response::Response {
    // Build a GET request with no body.
    let request = Request::builder()
        .method("GET")
        .uri(path)
        // GET requests have no body; `Body::empty()` satisfies the type without
        // allocating.
        .body(Body::empty())
        // request builder errors are programmer errors, not runtime conditions.
        .expect("request builder should not fail");

    // Drive the GET request through the service.
    app.oneshot(request)
        .await
        // A service-level error here is a bug, not an expected outcome.
        .expect("service call should not fail")
}

/// Read the response body as a `serde_json::Value`.
///
/// `usize::MAX` as the byte limit prevents accidental truncation of large
/// response bodies in tests where we might be verifying the full content.
async fn body_json(response: axum::response::Response) -> Value {
    // Read the complete response body — for tests, memory use is bounded.
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    // If this fails, the handler returned something that isn't valid JSON —
    // that itself is a test failure worth surfacing with the raw bytes.
    // Parse the bytes as JSON — a failure means the handler returned non-JSON.
    serde_json::from_slice(&bytes).expect("response body should be valid JSON")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn post_inbox_creates_item_and_returns_201() {
    // Happy path: valid JSON body, all required fields present.
    // Verifies that the handler creates the item, stores it, and returns
    // the full item in the response body with HTTP 201 Created.
    // Spin up a fresh test application with an isolated in-memory database.
    let app = build_test_app().await;

    // Issue the POST request and capture the response for assertion.
    let response = post_json(
        app,
        "/api/v1/inbox",
        json!({ "raw_text": "pick up pencils", "source": "touch" }),
    )
    .await;

    // 201 Created signals the item was stored; 200 OK would be incorrect here
    // because the semantics of POST-creating-a-resource require 201.
    assert_eq!(response.status(), StatusCode::CREATED);

    // Parse the response body for field-level assertions.
    let body = body_json(response).await;

    // Verify the response echoes back the fields we sent.
    assert_eq!(body["raw_text"], "pick up pencils");
    // Source must reflect what was sent in the request.
    assert_eq!(body["source"], "touch");
    // New items are always untriaged — the service must not default to any
    // other triage state.
    // triage_state must default to "untriaged" for all new captures.
    assert_eq!(body["triage_state"], "untriaged");

    // Server-generated fields: present and of the right JSON type.
    // Specific values can't be asserted (ID and timestamp are random/current).
    assert!(body["id"].is_string(), "id should be a string");
    // captured_at and captured_by are server-generated — present but non-deterministic.
    assert!(
        body["captured_at"].is_string(),
        "captured_at should be a string"
    );
    assert!(
        body["captured_by"].is_string(),
        "captured_by should be a string"
    );
}

#[tokio::test]
async fn post_inbox_empty_text_returns_422() {
    // The business rule "raw_text must not be empty" must be enforced at the
    // API boundary. Blank text is not a 400 (malformed JSON) — it is a 422
    // (well-formed but semantically invalid). The distinction matters because
    // 400 suggests the client sent bad syntax; 422 says the syntax was fine
    // but the value violated a constraint.
    // Fresh database — the rejection is purely logic-based, no stored data needed.
    let app = build_test_app().await;

    // Submit whitespace-only text; the business rule must reject it.
    let response = post_json(app, "/api/v1/inbox", json!({ "raw_text": "   " })).await;

    // 422 is the correct status for valid JSON that violates a business constraint.
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_inbox_source_defaults_to_touch() {
    // The `source` field is optional in the request; the server should default
    // it to "touch" when absent. This covers the common case where the hub
    // frontend doesn't bother sending a source field for tap events.
    // No source field in the request — the server must supply the default.
    let app = build_test_app().await;

    // Omit `source` entirely from the JSON body to exercise the default path.
    let response = post_json(
        app,
        "/api/v1/inbox",
        json!({ "raw_text": "a note without explicit source" }),
    )
    .await;

    // The missing source field must not cause a failure.
    assert_eq!(response.status(), StatusCode::CREATED);

    // Parse the response body to inspect the defaulted source field.
    let body = body_json(response).await;
    // Verify the server defaulted to "touch", not any other source variant.
    assert_eq!(body["source"], "touch");
}

#[tokio::test]
async fn get_recent_returns_empty_array_for_fresh_database() {
    // A brand-new database has no inbox items. The endpoint must return an
    // empty array, not a 404 or a 500. This is the expected state when the
    // service first starts up and no items have been captured yet.
    // No items in this database — the endpoint must handle zero rows gracefully.
    let app = build_test_app().await;

    // Query the recent list with no prior inserts.
    let response = get(app, "/api/v1/inbox/recent").await;

    // 200 + empty array is the contract for "inbox exists but has no items".
    // 404 would wrongly imply the resource doesn't exist.
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    // The response body must be a JSON array, not an object or null.
    assert!(body.is_array(), "body should be a JSON array");
    // `unwrap()` is safe here because we just asserted `is_array()`.
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn post_then_get_shows_created_item() {
    // End-to-end: create an item via POST, then verify it appears in the
    // GET /recent response. This exercises the full create→store→read path.
    //
    // The two app instances share the same `SqlitePool`, which means they
    // share the same underlying database connection and see each other's
    // writes immediately. This is the correct model for SQLite in WAL mode.
    // Both app instances share this pool so writes from app_post are visible to app_get.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");

    // Two routers backed by the same pool — simulating two sequential requests.
    let app_post = build_app(db.clone());
    let app_get = build_app(db);

    // POST to capture the item. The response is not asserted here — that's
    // covered by `post_inbox_creates_item_and_returns_201`.
    post_json(
        app_post,
        "/api/v1/inbox",
        json!({ "raw_text": "test note for round-trip", "source": "touch" }),
    )
    .await;

    // Now query for recent items; the just-posted item must appear.
    let response = get(app_get, "/api/v1/inbox/recent?limit=20").await;
    // Recent-list must return 200 OK.
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    // Extract the array for item-level assertions.
    let items = body.as_array().expect("array");
    // If the item wasn't persisted, this would be 0 instead of 1.
    assert_eq!(items.len(), 1, "expected one item after one POST");
    // Verify it's the same item we posted — not a stale item from another test.
    assert_eq!(items[0]["raw_text"], "test note for round-trip");
}

#[tokio::test]
async fn get_recent_limit_over_100_returns_400() {
    // The API contract caps `limit` at 100. Exceeding it is a client error
    // (400 Bad Request). We don't want the server to silently cap the limit —
    // that would hide a misuse that might indicate a client-side bug.
    // No items needed — the limit check is a pure request validation step.
    let app = build_test_app().await;

    // Query with limit=101, which exceeds the API cap of 100.
    let response = get(app, "/api/v1/inbox/recent?limit=101").await;
    // The status code must be 400, not 500 (this is a client error, not a server error).
    // 400 Bad Request (not 422) because this is a query-parameter constraint,
    // not a JSON body semantic validation.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_recent_default_limit_is_20() {
    // When `limit` is absent, the response should return at most 20 items.
    // This test inserts 5 (well under 20) to verify that "at most 20" means
    // "all items when there are fewer than 20", not "exactly 20".
    // Use a shared pool so the insert app and query app see the same items.
    let db = open_database("sqlite::memory:").await.expect("db");

    // Router::clone() shares the inner Arc — no connection data is copied.
    let app_insert = build_app(db.clone());
    // Insert 5 items, each via a separate oneshot request.
    for i in 0..5u32 {
        post_json(
            // Clone for each call so `oneshot` can take ownership.
            app_insert.clone(),
            "/api/v1/inbox",
            json!({ "raw_text": format!("item {i}") }),
        )
        .await;
    }

    // Query without a `limit` parameter — the server's default of 20 should apply.
    let app_query = build_app(db);
    // No `?limit=N` parameter in the URL.
    let response = get(app_query, "/api/v1/inbox/recent").await; // no limit param
    // The server must return 200 OK for a valid limit-less request.
    assert_eq!(response.status(), StatusCode::OK);

    // Parse the response body for item count assertion.
    let body = body_json(response).await;
    // Verify all 5 inserted items are returned (default limit > 5).
    let items = body.as_array().expect("array");
    // All 5 items should be returned because 5 < 20 (the default limit).
    assert_eq!(
        items.len(),
        5,
        "all 5 items should be returned under the default limit"
    );
}
