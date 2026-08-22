// grocery_repository.rs — integration tests for the grocery storage layer.
//
// Each test spins up a fresh in-memory SQLite database with all migrations
// applied and exercises the repository functions directly.
//
// What these tests verify:
//   • insert_grocery_list / fetch_grocery_list / list_grocery_lists round-trip.
//   • insert_grocery_item / list_grocery_items round-trip, including source
//     and source_meal_id.
//   • insert_grocery_items batch-inserts in one transaction.
//   • set_grocery_item_checked toggles the flag and returns whether a row matched.
//   • delete_grocery_item removes the row and returns whether one matched.

use amity_core::grocery::{GroceryItemBuilder, GroceryListBuilder, GrocerySource};
use amity_core::ids::{GroceryItemId, MealId};
use amity_storage::connection::open_database;
use amity_storage::grocery::{
    delete_grocery_item, fetch_grocery_list, insert_grocery_item, insert_grocery_items,
    insert_grocery_list, list_grocery_items, list_grocery_lists, set_grocery_item_checked,
};
use time::macros::datetime;

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-08-22 10:00:00 UTC)
}

#[tokio::test]
async fn insert_then_fetch_list_round_trips() {
    let pool = db().await;
    let list = GroceryListBuilder::new("Groceries")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_list(&pool, &list).await.unwrap();

    let fetched = fetch_grocery_list(&pool, list.id).await.unwrap().unwrap();
    assert_eq!(fetched.name, "Groceries");
    assert_eq!(fetched.created_at, now());

    let all = list_grocery_lists(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[tokio::test]
async fn insert_then_list_item_round_trips_all_fields() {
    let pool = db().await;
    let list = GroceryListBuilder::new("Groceries")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_list(&pool, &list).await.unwrap();

    let meal_id = MealId::new();
    let item = GroceryItemBuilder::new(list.id, "Flour")
        .qty("2 lb")
        .category("baking")
        .source(GrocerySource::FromMeal)
        .source_meal_id(meal_id)
        .now(now())
        .build()
        .unwrap();
    insert_grocery_item(&pool, &item).await.unwrap();

    let items = list_grocery_items(&pool, list.id).await.unwrap();
    assert_eq!(items.len(), 1);
    let fetched = &items[0];
    assert_eq!(fetched.name, "Flour");
    assert_eq!(fetched.qty.as_deref(), Some("2 lb"));
    assert_eq!(fetched.category.as_deref(), Some("baking"));
    assert!(!fetched.checked);
    assert_eq!(fetched.source, GrocerySource::FromMeal);
    assert_eq!(fetched.source_meal_id, Some(meal_id));
}

#[tokio::test]
async fn insert_grocery_items_batch_inserts_all() {
    let pool = db().await;
    let list = GroceryListBuilder::new("Groceries")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_list(&pool, &list).await.unwrap();

    let items = vec![
        GroceryItemBuilder::new(list.id, "Milk")
            .now(now())
            .build()
            .unwrap(),
        GroceryItemBuilder::new(list.id, "Eggs")
            .now(now())
            .build()
            .unwrap(),
    ];
    insert_grocery_items(&pool, &items).await.unwrap();

    let stored = list_grocery_items(&pool, list.id).await.unwrap();
    assert_eq!(stored.len(), 2);
}

#[tokio::test]
async fn set_checked_toggles_and_reports_match() {
    let pool = db().await;
    let list = GroceryListBuilder::new("Groceries")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_list(&pool, &list).await.unwrap();
    let item = GroceryItemBuilder::new(list.id, "Eggs")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_item(&pool, &item).await.unwrap();

    assert!(
        set_grocery_item_checked(&pool, item.id, true)
            .await
            .unwrap()
    );
    let items = list_grocery_items(&pool, list.id).await.unwrap();
    assert!(items[0].checked);

    assert!(
        set_grocery_item_checked(&pool, item.id, false)
            .await
            .unwrap()
    );
    let items = list_grocery_items(&pool, list.id).await.unwrap();
    assert!(!items[0].checked);

    // Unknown id reports no match.
    assert!(
        !set_grocery_item_checked(&pool, GroceryItemId::new(), true)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn delete_grocery_item_removes_row_and_reports_match() {
    let pool = db().await;
    let list = GroceryListBuilder::new("Groceries")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_list(&pool, &list).await.unwrap();
    let item = GroceryItemBuilder::new(list.id, "Eggs")
        .now(now())
        .build()
        .unwrap();
    insert_grocery_item(&pool, &item).await.unwrap();

    assert!(delete_grocery_item(&pool, item.id).await.unwrap());
    let items = list_grocery_items(&pool, list.id).await.unwrap();
    assert!(items.is_empty());

    assert!(!delete_grocery_item(&pool, item.id).await.unwrap());
}
