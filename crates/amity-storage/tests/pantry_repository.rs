// pantry_repository.rs — integration tests for the pantry storage layer.
//
// Each test spins up a fresh in-memory SQLite database with all migrations
// applied and exercises the repository functions directly.
//
// What these tests verify:
//   • insert_pantry_item / list_pantry_items round-trip, including the
//     optional note.
//   • delete_pantry_item removes the row and reports whether one matched.

use amity_core::pantry::PantryItemBuilder;
use amity_storage::connection::open_database;
use amity_storage::pantry::{delete_pantry_item, insert_pantry_item, list_pantry_items};
use time::macros::datetime;

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-08-22 10:00:00 UTC)
}

#[tokio::test]
async fn insert_then_list_round_trips_with_note() {
    let pool = db().await;
    let item = PantryItemBuilder::new("Flour")
        .note("always keep 2kg")
        .now(now())
        .build()
        .unwrap();
    insert_pantry_item(&pool, &item).await.unwrap();

    let all = list_pantry_items(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Flour");
    assert_eq!(all[0].note.as_deref(), Some("always keep 2kg"));
    assert_eq!(all[0].created_at, now());
}

#[tokio::test]
async fn insert_without_note_round_trips_none() {
    let pool = db().await;
    let item = PantryItemBuilder::new("Sugar").now(now()).build().unwrap();
    insert_pantry_item(&pool, &item).await.unwrap();

    let all = list_pantry_items(&pool).await.unwrap();
    assert!(all[0].note.is_none());
}

#[tokio::test]
async fn delete_pantry_item_removes_row_and_reports_match() {
    let pool = db().await;
    let item = PantryItemBuilder::new("Rice").now(now()).build().unwrap();
    insert_pantry_item(&pool, &item).await.unwrap();

    assert!(delete_pantry_item(&pool, item.id).await.unwrap());
    assert!(list_pantry_items(&pool).await.unwrap().is_empty());

    assert!(!delete_pantry_item(&pool, item.id).await.unwrap());
}
