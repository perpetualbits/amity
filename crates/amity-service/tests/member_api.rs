// member_api.rs — integration tests for the Member API.
//
// Mirrors `pantry_api.rs`'s harness: the full axum application over an
// in-memory database, driven with real HTTP requests via `oneshot`.
//
// What these tests verify:
//   • create → list → get → delete → get(404) round trip.
//   • Validation: a blank display_name is a 422.
//   • A malformed id is a 400 on GET.
//   • An unknown (but well-formed) id is a 404 on GET.
//   • GET /api/v1/members never includes the migration-0001 placeholder
//     sentinel (it would otherwise appear as a fake, selectable "Unnamed
//     member" cook/assignee in Slice 2's member picker).
//
// Note: migration 0001 seeds one placeholder member row, so list assertions
// check membership/presence rather than exact array length (see
// amity-storage/tests/member_repository.rs for the same caveat).

use amity_service::build_app;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── Harness (mirrors pantry_api.rs) ───────────────────────────────────────

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
async fn create_list_get_delete_round_trip() {
    let app = build_test_app().await;

    let create = post_json(
        app.clone(),
        "/api/v1/members",
        json!({ "display_name": "Alice", "initial": "A", "color": "sage" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert_eq!(created["display_name"], "Alice");
    assert_eq!(created["initial"], "A");
    assert_eq!(created["color"], "sage");
    let id = created["id"].as_str().expect("id").to_owned();

    let list = get(app.clone(), "/api/v1/members").await;
    assert_eq!(list.status(), StatusCode::OK);
    let listed = body_json(list).await;
    let members = listed["members"].as_array().expect("members array");
    assert!(members.iter().any(|m| m["id"] == id));
    // The legacy placeholder must never appear in the listed roster.
    assert!(
        members
            .iter()
            .all(|m| m["id"] != "00000000-0000-7000-8000-000000000001"),
        "placeholder sentinel must be excluded from GET /api/v1/members"
    );

    let fetched = get(app.clone(), &format!("/api/v1/members/{id}")).await;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(body_json(fetched).await["display_name"], "Alice");

    let deleted = delete(app.clone(), &format!("/api/v1/members/{id}")).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(body_json(deleted).await["deleted"], true);

    let after = get(app, &format!("/api/v1/members/{id}")).await;
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_members_excludes_placeholder_but_includes_real_members() {
    // Focused regression test for the sentinel-leak fix (see the module doc):
    // a real member must be listed; the migration-0001 placeholder must not.
    let app = build_test_app().await;
    let create = post_json(
        app.clone(),
        "/api/v1/members",
        json!({ "display_name": "Eve" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let id = body_json(create).await["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let list = get(app, "/api/v1/members").await;
    let members = body_json(list).await["members"]
        .as_array()
        .expect("members array")
        .clone();
    assert!(
        members.iter().any(|m| m["id"] == id),
        "real member must be present"
    );
    assert!(
        members
            .iter()
            .all(|m| m["id"] != "00000000-0000-7000-8000-000000000001"),
        "placeholder sentinel must be absent"
    );
}

#[tokio::test]
async fn optional_fields_are_omitted_when_absent() {
    let app = build_test_app().await;
    let create = post_json(app, "/api/v1/members", json!({ "display_name": "Ben" })).await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert!(created.get("initial").is_none());
    assert!(created.get("color").is_none());
}

// ─── Validation ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn blank_display_name_is_422() {
    let app = build_test_app().await;
    let resp = post_json(app, "/api/v1/members", json!({ "display_name": "   " })).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_member_bad_id_and_missing_id() {
    let app = build_test_app().await;

    let bad = get(app.clone(), "/api/v1/members/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    let missing = get(app, "/api/v1/members/018f1a2b-0000-7000-8000-000000000099").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
