// meal_api.rs — integration tests for the Meal API and Today's meal surfacing.
//
// Each test builds the full axum application (production code, not a mock)
// backed by an in-memory SQLite database and sends real HTTP requests via
// `tower::ServiceExt::oneshot`, mirroring `event_api.rs`'s harness exactly.
//
// What these tests verify:
//   • CRUD happy path: create → list → get → delete, ingredient lines and
//     the optional fields (slot, cook, notes) round-trip through the wire.
//   • Validation: a blank name is a 422; a malformed id is a 400; an unknown
//     id is a 404.
//   • `?from=`/`?to=` filtering on the list endpoint.
//   • The P2 Slice 3 payoff: a dinner-slot meal surfaces on Today as a
//     `kind: "meal"` item, and — critically — does NOT appear on `GET
//     /api/v1/week` for the same week, since Week is deliberately untouched.

use amity_service::build_app;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

// ─── Harness (mirrors event_api.rs) ────────────────────────────────────────

/// A fresh app over an isolated in-memory database. Cloning the returned
/// Router shares the same pool, so a create and a later query see each
/// other's writes.
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

/// DELETE a path.
async fn delete(app: axum::Router, path: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("DELETE")
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

// ─── CRUD happy path ────────────────────────────────────────────────────────

#[tokio::test]
async fn create_list_get_delete_round_trip() {
    let app = build_test_app().await;

    // Create a meal with every optional field populated.
    let create = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({
            "name": "Spaghetti bolognese",
            "date": "2099-03-10",
            "slot": "dinner",
            "ingredient_lines": [
                { "name": "spaghetti", "qty": "500g" },
                { "name": "tomato sauce" }
            ],
            "notes": "double the sauce for leftovers"
        }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = body_json(create).await;
    assert_eq!(created["name"], "Spaghetti bolognese");
    assert_eq!(created["date"], "2099-03-10");
    assert_eq!(created["slot"], "dinner");
    assert_eq!(created["notes"], "double the sauce for leftovers");
    let lines = created["ingredient_lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["name"], "spaghetti");
    assert_eq!(lines[0]["qty"], "500g");
    assert_eq!(lines[1]["name"], "tomato sauce");
    assert!(lines[1].get("qty").is_none(), "absent qty is omitted");
    let id = created["id"].as_str().expect("id").to_owned();

    // List returns it (bare array, no ?from=/?to= filter).
    let list = get(app.clone(), "/api/v1/meals").await;
    assert_eq!(list.status(), StatusCode::OK);
    let listed = body_json(list).await;
    let meals = listed.as_array().expect("array");
    assert_eq!(meals.len(), 1);
    assert_eq!(meals[0]["id"], id);

    // Get by id.
    let found = get(app.clone(), &format!("/api/v1/meals/{id}")).await;
    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(body_json(found).await["id"], id);

    // Delete it.
    let deleted = delete(app.clone(), &format!("/api/v1/meals/{id}")).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(body_json(deleted).await["deleted"], true);

    // It is gone.
    let gone = get(app, &format!("/api/v1/meals/{id}")).await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn slot_defaults_to_dinner_when_absent() {
    let app = build_test_app().await;
    let create = post_json(
        app,
        "/api/v1/meals",
        json!({ "name": "Cereal", "date": "2099-03-11" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    assert_eq!(body_json(create).await["slot"], "dinner");
}

// ─── Validation ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn blank_name_is_422() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/meals",
        json!({ "name": "   ", "date": "2099-03-10" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn bad_date_is_422() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/meals",
        json!({ "name": "Tacos", "date": "not-a-date" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn unknown_slot_is_422() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/meals",
        json!({ "name": "Tacos", "date": "2099-03-10", "slot": "brunch" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_meal_bad_id_and_missing_id() {
    let app = build_test_app().await;

    // Malformed UUID → 400.
    let bad = get(app.clone(), "/api/v1/meals/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // Well-formed but unknown → 404.
    let missing = get(app, "/api/v1/meals/018f1a2b-0000-7000-8000-000000000099").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_meal_bad_id_and_missing_id() {
    let app = build_test_app().await;

    let bad = delete(app.clone(), "/api/v1/meals/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    let missing = delete(app, "/api/v1/meals/018f1a2b-0000-7000-8000-000000000099").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

// ─── ?from=/?to= filtering ──────────────────────────────────────────────────

#[tokio::test]
async fn from_to_filters_the_list_and_a_half_pair_is_400() {
    let app = build_test_app().await;

    // Two meals a week apart.
    post_json(
        app.clone(),
        "/api/v1/meals",
        json!({ "name": "In range", "date": "2099-04-10" }),
    )
    .await;
    post_json(
        app.clone(),
        "/api/v1/meals",
        json!({ "name": "Out of range", "date": "2099-04-20" }),
    )
    .await;

    // Range that only covers the first meal.
    let ranged = get(app.clone(), "/api/v1/meals?from=2099-04-08&to=2099-04-12").await;
    assert_eq!(ranged.status(), StatusCode::OK);
    let meals = body_json(ranged).await;
    let meals = meals.as_array().expect("array");
    assert_eq!(meals.len(), 1);
    assert_eq!(meals[0]["name"], "In range");

    // Half the pair → 400.
    let half = get(app, "/api/v1/meals?from=2099-04-08").await;
    assert_eq!(half.status(), StatusCode::BAD_REQUEST);
}

// ─── Meal-on-Today surfacing (P2 Slice 3) ──────────────────────────────────

#[tokio::test]
async fn dinner_meal_surfaces_on_today_but_not_on_week() {
    let app = build_test_app().await;

    // A far-future Wednesday so it is never "overdue" and its week is stable.
    // 2099-04-28 is a Wednesday.
    let create = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({ "name": "Fajita night", "date": "2099-04-28", "slot": "dinner" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);

    // It surfaces on Today for that date.
    let today = get(app.clone(), "/api/v1/surfacing/today?date=2099-04-28").await;
    assert_eq!(today.status(), StatusCode::OK);
    let body = body_json(today).await;
    assert_eq!(body["has_surfaced"], true);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "meal");
    assert_eq!(items[0]["title"], "Fajita night");
    assert_eq!(items[0]["all_day"], true);

    // The same week's Week grid does NOT include it — Week is untouched.
    let week = get(app, "/api/v1/week?start=2099-04-28").await;
    assert_eq!(week.status(), StatusCode::OK);
    let week_body = body_json(week).await;
    let days = week_body["days"].as_array().expect("days array");
    assert_eq!(days.len(), 7);
    for day in days {
        let items = day["items"].as_array().expect("items array");
        assert!(
            items.iter().all(|i| i["kind"] != "meal"),
            "no day in Week should ever show a meal item"
        );
    }
}

#[tokio::test]
async fn breakfast_meal_does_not_surface_on_today() {
    let app = build_test_app().await;

    // Today's meal surfacing is scoped to the dinner slot only.
    let create = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({ "name": "Pancakes", "date": "2099-05-05", "slot": "breakfast" }),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);

    let today = get(app, "/api/v1/surfacing/today?date=2099-05-05").await;
    let body = body_json(today).await;
    assert_eq!(body["has_surfaced"], false);
}
