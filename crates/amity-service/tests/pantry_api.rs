// pantry_api.rs — integration tests for the Pantry API.
//
// Mirrors `event_api.rs`'s harness: the full axum application over an
// in-memory database, driven with real HTTP requests via `oneshot`.
//
// What these tests verify:
//   • CRUD happy path: create → list → delete, note round-trips.
//   • Validation: a blank name is a 422.
//   • A malformed id is a 400; an unknown id is a 404.

use amity_service::build_app;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── Harness (mirrors event_api.rs) ────────────────────────────────────────

async fn build_test_app() -> axum::Router {
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should open");
    build_app(db)
}

async fn post_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

async fn get(app: axum::Router, path: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

async fn delete(app: axum::Router, path: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("DELETE")
        .uri(path)
        .body(Body::empty())
        .expect("request builds");
    app.oneshot(request).await.expect("service call")
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body readable");
    serde_json::from_slice(&bytes).expect("valid JSON")
}

// ─── CRUD happy path ────────────────────────────────────────────────────────

#[tokio::test]
async fn create_list_delete_round_trip() {
    let app = build_test_app().await;

    let create = post_json(
        app.clone(),
        "/api/v1/pantry",
        json!({ "name": "Flour", "note": "keep two bags, we bake a lot" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert_eq!(created["name"], "Flour");
    assert_eq!(created["note"], "keep two bags, we bake a lot");
    let id = created["id"].as_str().expect("id").to_owned();

    let list = get(app.clone(), "/api/v1/pantry").await;
    assert_eq!(list.status(), StatusCode::OK);
    let items = body_json(list).await;
    let items = items.as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id);

    let deleted = delete(app.clone(), &format!("/api/v1/pantry/{id}")).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(body_json(deleted).await["deleted"], true);

    let list_after = get(app, "/api/v1/pantry").await;
    let items = body_json(list_after).await;
    assert!(items.as_array().expect("array").is_empty());
}

#[tokio::test]
async fn note_is_omitted_when_absent() {
    let app = build_test_app().await;
    let create = post_json(app, "/api/v1/pantry", json!({ "name": "Rice" })).await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert!(created.get("note").is_none());
}

// ─── Validation ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn blank_name_is_422() {
    let app = build_test_app().await;
    let resp = post_json(app, "/api/v1/pantry", json!({ "name": "   " })).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn delete_pantry_item_bad_id_and_missing_id() {
    let app = build_test_app().await;

    let bad = delete(app.clone(), "/api/v1/pantry/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    let missing = delete(app, "/api/v1/pantry/018f1a2b-0000-7000-8000-000000000099").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
