// event_repository.rs — integration tests for the Event storage repositories.
//
// Each test runs against a fresh in-memory SQLite database with all migrations
// applied, exercising the real repository functions (no mocks). They verify the
// full round-trip: domain Event -> insert -> fetch/list -> domain Event, plus
// event instances and overrides.

use amity_core::event::{Event, EventBuilder, EventSource, EventSourceKind};
use amity_core::event_override::{EventOverride, OverrideAction};
use amity_core::ids::{EventId, MemberId};
use amity_core::recurrence::RecurrenceRule;
use amity_storage::connection::open_database;
use amity_storage::event::{fetch_event, insert_event, list_events, update_event};
use amity_storage::event_instance::{
    EventInstance, delete_future_event_instances, list_upcoming_event_instances,
    prune_old_event_instances, upsert_event_instances,
};
use amity_storage::event_override::{insert_event_override, list_overrides_on_date};
use sqlx::SqlitePool;
use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// A fresh in-memory database with migrations applied.
async fn fresh_db() -> SqlitePool {
    open_database("sqlite::memory:")
        .await
        .expect("in-memory database should open")
}

/// The placeholder member seeded by migration 0001.
fn member() -> MemberId {
    MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
}

/// A fixed "now" for deterministic timestamps.
fn now() -> OffsetDateTime {
    datetime!(2026-07-25 10:00:00 UTC)
}

/// Build a minimal timed native event starting at `start`.
fn timed_event(title: &str, start: OffsetDateTime) -> Event {
    EventBuilder::new()
        .title(title)
        .start_at(start)
        .now(now())
        .build()
        .expect("valid event")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn insert_and_fetch_round_trips_a_native_event() {
    let db = fresh_db().await;
    // A timed native event with an end and a location.
    let event = EventBuilder::new()
        .title("school play")
        .start_at(datetime!(2026-07-26 19:00:00 +02:00))
        .end_at(datetime!(2026-07-26 21:00:00 +02:00))
        .location("main hall")
        .now(now())
        .build()
        .unwrap();

    insert_event(&db, &event).await.expect("insert");
    let fetched = fetch_event(&db, event.id)
        .await
        .expect("fetch")
        .expect("some");

    // The whole struct round-trips (PartialEq compares every field).
    assert_eq!(fetched, event);
    // Native events are editable.
    assert_eq!(fetched.source.kind, EventSourceKind::Native);
    assert!(!fetched.source.read_only);
}

#[tokio::test]
async fn fetch_missing_event_returns_none() {
    let db = fresh_db().await;
    let missing = fetch_event(&db, EventId::new()).await.expect("fetch");
    assert!(missing.is_none());
}

#[tokio::test]
async fn external_and_recurring_events_round_trip() {
    let db = fresh_db().await;

    // A read-only external all-day event.
    let external = EventBuilder::new()
        .title("term starts")
        .start_at(datetime!(2026-09-01 00:00:00 +02:00))
        .all_day(true)
        .source(EventSource::ics("ext-1", "school-cal", now()))
        .now(now())
        .build()
        .unwrap();
    insert_event(&db, &external).await.expect("insert external");

    // A recurring native event.
    let recurring = EventBuilder::new()
        .title("bin day")
        .start_at(datetime!(2026-07-27 07:00:00 +02:00))
        .recurrence(RecurrenceRule::new(
            "FREQ=WEEKLY;BYDAY=MO",
            "Europe/Amsterdam",
        ))
        .now(now())
        .build()
        .unwrap();
    insert_event(&db, &recurring)
        .await
        .expect("insert recurring");

    // Both come back byte-for-byte through fetch.
    assert_eq!(
        fetch_event(&db, external.id).await.unwrap().unwrap(),
        external
    );
    assert_eq!(
        fetch_event(&db, recurring.id).await.unwrap().unwrap(),
        recurring
    );
}

#[tokio::test]
async fn list_events_returns_all_ordered_by_start() {
    let db = fresh_db().await;
    // Insert two events out of chronological order.
    let later = timed_event("later", datetime!(2026-07-28 09:00:00 UTC));
    let earlier = timed_event("earlier", datetime!(2026-07-27 09:00:00 UTC));
    insert_event(&db, &later).await.unwrap();
    insert_event(&db, &earlier).await.unwrap();

    let events = list_events(&db).await.expect("list");
    // Ordered by start_at ascending, regardless of insert order.
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].title, "earlier");
    assert_eq!(events[1].title, "later");
}

#[tokio::test]
async fn update_event_changes_fields() {
    let db = fresh_db().await;
    let mut event = timed_event("draft", datetime!(2026-07-29 09:00:00 UTC));
    insert_event(&db, &event).await.unwrap();

    // Change the title and bump updated_at, then persist.
    event.title = "final".to_owned();
    event.updated_at = datetime!(2026-07-25 11:00:00 UTC);
    update_event(&db, &event).await.expect("update");

    let fetched = fetch_event(&db, event.id).await.unwrap().unwrap();
    assert_eq!(fetched.title, "final");
}

#[tokio::test]
async fn event_instances_upsert_list_prune_and_delete() {
    let db = fresh_db().await;
    // A parent event to satisfy the instance foreign key.
    let event = timed_event("standup", datetime!(2026-07-27 09:00:00 UTC));
    insert_event(&db, &event).await.unwrap();

    // Three instances: one aged-out, two upcoming.
    let aged = EventInstance {
        id: "018f0000-0000-7000-8000-0000000000a1".to_owned(),
        event_id: event.id,
        scheduled_at: now() - Duration::days(40),
    };
    let soon = EventInstance {
        id: "018f0000-0000-7000-8000-0000000000a2".to_owned(),
        event_id: event.id,
        scheduled_at: now() + Duration::days(1),
    };
    let later = EventInstance {
        id: "018f0000-0000-7000-8000-0000000000a3".to_owned(),
        event_id: event.id,
        scheduled_at: now() + Duration::days(5),
    };
    upsert_event_instances(&db, &[aged.clone(), soon, later])
        .await
        .expect("upsert");
    // Re-upserting the same set is idempotent (INSERT OR IGNORE).
    upsert_event_instances(&db, std::slice::from_ref(&aged))
        .await
        .unwrap();

    // Upcoming from `now` returns the two future instances.
    let upcoming = list_upcoming_event_instances(&db, event.id, now(), 100)
        .await
        .unwrap();
    assert_eq!(upcoming.len(), 2);

    // Prune everything older than 30 days ago → removes the aged-out one.
    let pruned = prune_old_event_instances(&db, now() - Duration::days(30))
        .await
        .unwrap();
    assert_eq!(pruned, 1);

    // Delete future instances from now → removes the two upcoming ones.
    let deleted = delete_future_event_instances(&db, event.id, now())
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    // Nothing left from a wide lower bound.
    let remaining = list_upcoming_event_instances(&db, event.id, now() - Duration::days(100), 100)
        .await
        .unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn overrides_insert_and_list_by_date() {
    let db = fresh_db().await;
    // A parent event to satisfy the override foreign key.
    let event = timed_event("bin day", datetime!(2026-04-27 07:00:00 +02:00));
    insert_event(&db, &event).await.unwrap();

    // A cancel override on King's Day, and an annotate on another date.
    let cancel = EventOverride::new(
        event.id,
        date!(2026 - 04 - 27),
        OverrideAction::Cancel,
        None,
        member(),
        now(),
    );
    let annotate = EventOverride::new(
        event.id,
        date!(2026 - 05 - 04),
        OverrideAction::Annotate,
        Some("moved to the shed".to_owned()),
        member(),
        now(),
    );
    insert_event_override(&db, &cancel)
        .await
        .expect("insert cancel");
    insert_event_override(&db, &annotate)
        .await
        .expect("insert annotate");

    // Listing by King's Day returns only the cancel override, round-tripped.
    let on_kings_day = list_overrides_on_date(&db, date!(2026 - 04 - 27))
        .await
        .unwrap();
    assert_eq!(on_kings_day.len(), 1);
    assert_eq!(on_kings_day[0], cancel);
    assert_eq!(on_kings_day[0].action, OverrideAction::Cancel);
}
