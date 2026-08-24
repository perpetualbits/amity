// grocery_api.rs — integration tests for the Grocery List/Item API and the
// generation endpoint (P2 Slice 3's payoff).
//
// Mirrors `event_api.rs`'s harness: the full axum application over an
// in-memory database, driven with real HTTP requests via `oneshot`.
//
// What these tests verify:
//   • CRUD happy path for lists and items (create/list/get, PATCH checked,
//     DELETE).
//   • Validation: a blank name is a 422; a malformed id is a 400; an unknown
//     id is a 404.
//   • Generation e2e: seeding meals with ingredient lines plus a pantry
//     staple, then POSTing `/generate`, produces additions that exclude the
//     pantry staple and are `source: "from_meal"`.
//   • The no-clobber property, end to end: after checking one generated item
//     and adding a manual item, a second `/generate` call leaves the checked
//     state untouched, keeps the manual item, and adds no duplicates.

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

async fn patch_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    let request = Request::builder()
        .method("PATCH")
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

/// Create a grocery list and return its id.
async fn create_list(app: axum::Router, name: &str) -> String {
    let resp = post_json(app, "/api/v1/grocery-lists", json!({ "name": name })).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    body_json(resp).await["id"].as_str().unwrap().to_owned()
}

// ─── CRUD happy path: lists ─────────────────────────────────────────────────

#[tokio::test]
async fn create_list_and_get_and_list_round_trip() {
    let app = build_test_app().await;
    let id = create_list(app.clone(), "Groceries").await;

    let list_all = get(app.clone(), "/api/v1/grocery-lists").await;
    assert_eq!(list_all.status(), StatusCode::OK);
    let lists = body_json(list_all).await;
    assert_eq!(lists.as_array().expect("array").len(), 1);

    let found = get(app, &format!("/api/v1/grocery-lists/{id}")).await;
    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(body_json(found).await["name"], "Groceries");
}

#[tokio::test]
async fn create_list_blank_name_is_422() {
    let app = build_test_app().await;
    let resp = post_json(app, "/api/v1/grocery-lists", json!({ "name": "   " })).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_list_bad_id_and_missing_id() {
    let app = build_test_app().await;
    let bad = get(app.clone(), "/api/v1/grocery-lists/not-a-uuid").await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let missing = get(
        app,
        "/api/v1/grocery-lists/018f1a2b-0000-7000-8000-000000000099",
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

// ─── CRUD happy path: items ─────────────────────────────────────────────────

#[tokio::test]
async fn manual_item_add_list_patch_delete_round_trip() {
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;

    // Manual add.
    let add = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/items"),
        json!({ "name": "Eggs", "qty": "1 dozen", "category": "dairy" }),
    )
    .await;
    assert_eq!(add.status(), StatusCode::CREATED);
    let item = body_json(add).await;
    assert_eq!(item["name"], "Eggs");
    assert_eq!(item["source"], "manual");
    assert_eq!(item["checked"], false);
    assert!(item.get("source_meal_id").is_none());
    let item_id = item["id"].as_str().unwrap().to_owned();

    // List items.
    let items = get(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/items"),
    )
    .await;
    assert_eq!(items.status(), StatusCode::OK);
    let items = body_json(items).await;
    assert_eq!(items.as_array().expect("array").len(), 1);

    // Patch checked.
    let patched = patch_json(
        app.clone(),
        &format!("/api/v1/grocery-items/{item_id}"),
        json!({ "checked": true }),
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    assert_eq!(body_json(patched).await["checked"], true);

    // Confirm it stuck.
    let items = get(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/items"),
    )
    .await;
    let items = body_json(items).await;
    assert_eq!(items[0]["checked"], true);

    // Delete it.
    let deleted = delete(app.clone(), &format!("/api/v1/grocery-items/{item_id}")).await;
    assert_eq!(deleted.status(), StatusCode::OK);

    let items = get(app, &format!("/api/v1/grocery-lists/{list_id}/items")).await;
    let items = body_json(items).await;
    assert!(items.as_array().expect("array").is_empty());
}

#[tokio::test]
async fn add_item_to_missing_list_is_404() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/grocery-lists/018f1a2b-0000-7000-8000-000000000099/items",
        json!({ "name": "Eggs" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn add_item_blank_name_is_422() {
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;
    let resp = post_json(
        app,
        &format!("/api/v1/grocery-lists/{list_id}/items"),
        json!({ "name": "   " }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn patch_and_delete_item_bad_id_and_missing_id() {
    let app = build_test_app().await;

    let bad_patch = patch_json(
        app.clone(),
        "/api/v1/grocery-items/not-a-uuid",
        json!({ "checked": true }),
    )
    .await;
    assert_eq!(bad_patch.status(), StatusCode::BAD_REQUEST);

    let missing_patch = patch_json(
        app.clone(),
        "/api/v1/grocery-items/018f1a2b-0000-7000-8000-000000000099",
        json!({ "checked": true }),
    )
    .await;
    assert_eq!(missing_patch.status(), StatusCode::NOT_FOUND);

    let bad_delete = delete(app.clone(), "/api/v1/grocery-items/not-a-uuid").await;
    assert_eq!(bad_delete.status(), StatusCode::BAD_REQUEST);

    let missing_delete = delete(
        app,
        "/api/v1/grocery-items/018f1a2b-0000-7000-8000-000000000099",
    )
    .await;
    assert_eq!(missing_delete.status(), StatusCode::NOT_FOUND);
}

// ─── Generation e2e (the phase's payoff) ────────────────────────────────────

#[tokio::test]
async fn generate_excludes_pantry_staples_and_tags_from_meal() {
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;

    // A pantry staple that must never be suggested.
    let pantry = post_json(app.clone(), "/api/v1/pantry", json!({ "name": "Flour" })).await;
    assert_eq!(pantry.status(), StatusCode::CREATED);

    // A meal in the target range with ingredient lines, one matching the
    // pantry staple (case-insensitively).
    let meal = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({
            "name": "Bread",
            "date": "2099-06-10",
            "ingredient_lines": [
                { "name": "flour" },
                { "name": "yeast", "qty": "1 packet" }
            ]
        }),
    )
    .await;
    assert_eq!(meal.status(), StatusCode::CREATED);

    // Generate over a range covering the meal's date.
    let generate = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/generate?from=2099-06-08&to=2099-06-12"),
        json!({}),
    )
    .await;
    assert_eq!(generate.status(), StatusCode::OK);
    let body = body_json(generate).await;
    assert_eq!(body["from"], "2099-06-08");
    assert_eq!(body["to"], "2099-06-12");
    let added = body["added"].as_array().expect("added array");
    assert_eq!(added.len(), 1, "flour must be suppressed by the pantry");
    assert_eq!(added[0]["name"], "yeast");
    assert_eq!(added[0]["source"], "from_meal");
    assert!(added[0].get("source_meal_id").is_some());

    // The addition was actually persisted.
    let items = get(app, &format!("/api/v1/grocery-lists/{list_id}/items")).await;
    let items = body_json(items).await;
    let items = items.as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "yeast");
}

#[tokio::test]
async fn generate_no_clobber_end_to_end() {
    // The no-clobber property: re-generating after a manual check must NOT
    // uncheck/remove existing items, and must NOT duplicate already-present
    // lines.
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;

    // One meal, two ingredients, no pantry overlap.
    let meal = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({
            "name": "Omelette",
            "date": "2099-07-14",
            "ingredient_lines": [
                { "name": "eggs", "qty": "6" },
                { "name": "chives" }
            ]
        }),
    )
    .await;
    assert_eq!(meal.status(), StatusCode::CREATED);

    let range = "from=2099-07-12&to=2099-07-16";

    // First generation: both ingredients are added.
    let first = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/generate?{range}"),
        json!({}),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = body_json(first).await;
    let first_added = first_body["added"].as_array().expect("added array");
    assert_eq!(first_added.len(), 2);

    // Find the generated "eggs" item and check it off.
    let items_resp = get(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/items"),
    )
    .await;
    let items = body_json(items_resp).await;
    let items = items.as_array().expect("array");
    assert_eq!(items.len(), 2);
    let eggs_id = items
        .iter()
        .find(|i| i["name"] == "eggs")
        .expect("eggs item present")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let checked = patch_json(
        app.clone(),
        &format!("/api/v1/grocery-items/{eggs_id}"),
        json!({ "checked": true }),
    )
    .await;
    assert_eq!(checked.status(), StatusCode::OK);

    // Add a manual item too.
    let manual = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/items"),
        json!({ "name": "napkins" }),
    )
    .await;
    assert_eq!(manual.status(), StatusCode::CREATED);

    // Second generation over the SAME range: no-clobber must hold.
    let second = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/generate?{range}"),
        json!({}),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = body_json(second).await;
    let second_added = second_body["added"].as_array().expect("added array");
    assert!(
        second_added.is_empty(),
        "eggs and chives are already on the list; nothing new to add"
    );

    // Final state: exactly 3 items — eggs (still checked), chives
    // (untouched), napkins (manual, survives) — no duplicates.
    let final_items = get(app, &format!("/api/v1/grocery-lists/{list_id}/items")).await;
    let final_items = body_json(final_items).await;
    let final_items = final_items.as_array().expect("array");
    assert_eq!(
        final_items.len(),
        3,
        "no duplicates after a second generation"
    );

    let eggs = final_items
        .iter()
        .find(|i| i["name"] == "eggs")
        .expect("eggs still present");
    assert_eq!(eggs["checked"], true, "checked state must survive");

    let chives = final_items
        .iter()
        .find(|i| i["name"] == "chives")
        .expect("chives still present");
    assert_eq!(chives["checked"], false);

    let napkins = final_items
        .iter()
        .find(|i| i["name"] == "napkins")
        .expect("manual item survives");
    assert_eq!(napkins["source"], "manual");
}

#[tokio::test]
async fn clear_checked_then_regenerate_readds_item() {
    // Slice 3's motivating interaction: a checked (bought) item still sitting
    // on the list blocks its own re-addition. "Clear checked" lets the
    // household reset the list; after clearing, a checked-off line is no
    // longer present, so a later generation can legitimately re-add it — but
    // a pantry staple stays suppressed throughout, since that suppression
    // never depended on what was on the list.
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;

    // A pantry staple that must stay suppressed across the whole scenario.
    let pantry = post_json(app.clone(), "/api/v1/pantry", json!({ "name": "flour" })).await;
    assert_eq!(pantry.status(), StatusCode::CREATED);

    // One meal: two ingredients that should reach the list, one that never
    // should (the pantry staple).
    let meal = post_json(
        app.clone(),
        "/api/v1/meals",
        json!({
            "name": "Pancakes",
            "date": "2099-08-11",
            "ingredient_lines": [
                { "name": "eggs" },
                { "name": "chives" },
                { "name": "flour" }
            ]
        }),
    )
    .await;
    assert_eq!(meal.status(), StatusCode::CREATED);

    let range = "from=2099-08-09&to=2099-08-13";

    // First generation: eggs and chives are added; flour is suppressed.
    let first = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/generate?{range}"),
        json!({}),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_added = body_json(first).await["added"]
        .as_array()
        .expect("added array")
        .len();
    assert_eq!(first_added, 2, "flour must be suppressed by the pantry");

    // Check "eggs" off (bought).
    let items = body_json(
        get(
            app.clone(),
            &format!("/api/v1/grocery-lists/{list_id}/items"),
        )
        .await,
    )
    .await;
    let items = items.as_array().expect("array");
    let eggs_id = items
        .iter()
        .find(|i| i["name"] == "eggs")
        .expect("eggs item present")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let checked = patch_json(
        app.clone(),
        &format!("/api/v1/grocery-items/{eggs_id}"),
        json!({ "checked": true }),
    )
    .await;
    assert_eq!(checked.status(), StatusCode::OK);

    // Clear checked items: only eggs is checked, so exactly one is removed.
    let cleared = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/clear-checked"),
        json!({}),
    )
    .await;
    assert_eq!(cleared.status(), StatusCode::OK);
    assert_eq!(body_json(cleared).await["removed"], 1);

    // eggs is gone; chives (unchecked) remains.
    let after_clear = body_json(
        get(
            app.clone(),
            &format!("/api/v1/grocery-lists/{list_id}/items"),
        )
        .await,
    )
    .await;
    let after_clear = after_clear.as_array().expect("array");
    assert_eq!(after_clear.len(), 1, "only the checked item is removed");
    assert_eq!(after_clear[0]["name"], "chives");
    assert_eq!(after_clear[0]["checked"], false);

    // Generate again over the same range: eggs is no longer present, so it
    // is legitimately re-added; chives is already there so no duplicate;
    // flour is still suppressed by the pantry.
    let second = post_json(
        app.clone(),
        &format!("/api/v1/grocery-lists/{list_id}/generate?{range}"),
        json!({}),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_added = body_json(second).await;
    let second_added = second_added["added"].as_array().expect("added array");
    assert_eq!(second_added.len(), 1, "only eggs is re-added");
    assert_eq!(second_added[0]["name"], "eggs");

    let final_items =
        body_json(get(app, &format!("/api/v1/grocery-lists/{list_id}/items")).await).await;
    let final_items = final_items.as_array().expect("array");
    assert_eq!(final_items.len(), 2, "eggs re-added, chives still there");
    assert!(final_items.iter().any(|i| i["name"] == "eggs"));
    assert!(final_items.iter().any(|i| i["name"] == "chives"));
    assert!(
        !final_items.iter().any(|i| i["name"] == "flour"),
        "flour stays suppressed by the pantry"
    );
}

#[tokio::test]
async fn clear_checked_missing_list_is_404() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/grocery-lists/018f1a2b-0000-7000-8000-000000000099/clear-checked",
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn clear_checked_bad_list_id_is_400() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/grocery-lists/not-a-uuid/clear-checked",
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn generate_missing_list_is_404() {
    let app = build_test_app().await;
    let resp = post_json(
        app,
        "/api/v1/grocery-lists/018f1a2b-0000-7000-8000-000000000099/generate",
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn generate_half_date_pair_is_400() {
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;
    let resp = post_json(
        app,
        &format!("/api/v1/grocery-lists/{list_id}/generate?from=2099-06-08"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn generate_defaults_to_current_week_when_no_range_given() {
    // With no meals in the current week, generating with no ?from=/?to= must
    // succeed with an empty `added`, proving the default-week fallback runs
    // without error.
    let app = build_test_app().await;
    let list_id = create_list(app.clone(), "Groceries").await;
    let resp = post_json(
        app,
        &format!("/api/v1/grocery-lists/{list_id}/generate"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["added"].as_array().expect("added array").is_empty());
    // The echoed range spans exactly 7 days (Monday..Sunday).
    assert!(body["from"].is_string());
    assert!(body["to"].is_string());
}
