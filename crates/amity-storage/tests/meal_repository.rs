// meal_repository.rs — integration tests for the meal storage layer.
//
// Each test spins up a fresh in-memory SQLite database with all migrations
// applied and exercises the repository functions directly.
//
// What these tests verify:
//   • insert_meal / fetch_meal: every field survives the write→read cycle,
//     including ingredient_lines IN ORDER.
//   • list_meals / list_meals_in_range: all meals vs. a date-bounded subset.
//   • delete_meal: removes the meal AND its ingredient rows (no orphans).

use amity_core::ids::{MealId, MemberId};
use amity_core::meal::{MealBuilder, MealSlot};
use amity_storage::connection::open_database;
use amity_storage::meal::{delete_meal, fetch_meal, insert_meal, list_meals, list_meals_in_range};
use time::macros::{date, datetime};

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-08-22 10:00:00 UTC)
}

fn placeholder_member() -> MemberId {
    MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
}

#[tokio::test]
async fn insert_then_fetch_round_trips_all_fields_and_ingredient_order() {
    let pool = db().await;
    let meal = MealBuilder::new("Tacos")
        .date(date!(2026 - 08 - 24))
        .slot(MealSlot::Lunch)
        .cook(placeholder_member())
        .ingredient("beef", Some("1 lb".to_owned()))
        .ingredient("tortillas", None)
        .ingredient("salsa", Some("1 jar".to_owned()))
        .notes("kids' favourite")
        .now(now())
        .build()
        .unwrap();

    insert_meal(&pool, &meal).await.unwrap();

    let fetched = fetch_meal(&pool, meal.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, meal.id);
    assert_eq!(fetched.date, meal.date);
    assert_eq!(fetched.slot, MealSlot::Lunch);
    assert_eq!(fetched.name, "Tacos");
    assert_eq!(fetched.cook, Some(placeholder_member()));
    assert_eq!(fetched.notes.as_deref(), Some("kids' favourite"));
    assert_eq!(fetched.created_at, now());

    // Ingredient order must be preserved exactly as inserted.
    let names: Vec<&str> = fetched
        .ingredient_lines
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, vec!["beef", "tortillas", "salsa"]);
    assert_eq!(fetched.ingredient_lines[0].qty.as_deref(), Some("1 lb"));
    assert!(fetched.ingredient_lines[1].qty.is_none());
}

#[tokio::test]
async fn fetch_unknown_meal_returns_none() {
    let pool = db().await;
    let result = fetch_meal(&pool, MealId::new()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn list_meals_returns_all_meals() {
    let pool = db().await;
    let a = MealBuilder::new("Pancakes")
        .date(date!(2026 - 08 - 24))
        .now(now())
        .build()
        .unwrap();
    let b = MealBuilder::new("Stir fry")
        .date(date!(2026 - 08 - 25))
        .now(now())
        .build()
        .unwrap();
    insert_meal(&pool, &a).await.unwrap();
    insert_meal(&pool, &b).await.unwrap();

    let all = list_meals(&pool).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn list_meals_in_range_filters_by_date() {
    let pool = db().await;
    let inside = MealBuilder::new("Inside")
        .date(date!(2026 - 08 - 24))
        .now(now())
        .build()
        .unwrap();
    let before = MealBuilder::new("Before")
        .date(date!(2026 - 08 - 20))
        .now(now())
        .build()
        .unwrap();
    let after = MealBuilder::new("After")
        .date(date!(2026 - 09 - 01))
        .now(now())
        .build()
        .unwrap();
    insert_meal(&pool, &inside).await.unwrap();
    insert_meal(&pool, &before).await.unwrap();
    insert_meal(&pool, &after).await.unwrap();

    let in_range = list_meals_in_range(&pool, date!(2026 - 08 - 22), date!(2026 - 08 - 28))
        .await
        .unwrap();
    assert_eq!(in_range.len(), 1);
    assert_eq!(in_range[0].name, "Inside");
}

#[tokio::test]
async fn delete_meal_removes_ingredients_and_the_meal_row() {
    let pool = db().await;
    let meal = MealBuilder::new("Curry")
        .date(date!(2026 - 08 - 24))
        .ingredient("rice", None)
        .ingredient("curry paste", Some("1 tbsp".to_owned()))
        .now(now())
        .build()
        .unwrap();
    insert_meal(&pool, &meal).await.unwrap();

    assert!(delete_meal(&pool, meal.id).await.unwrap());
    assert!(fetch_meal(&pool, meal.id).await.unwrap().is_none());

    // No orphaned ingredient rows left behind.
    let orphans: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meal_ingredients")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(orphans, 0);

    // Deleting again reports no match.
    assert!(!delete_meal(&pool, meal.id).await.unwrap());
}
