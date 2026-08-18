// calendar_repository.rs — integration tests for the calendars repository and
// the external-event upsert/prune helpers, over a fresh in-memory database.

use amity_core::calendar::{CalendarBuilder, CalendarCategory, CalendarSyncState, SyncStatus};
use amity_core::event::{EventBuilder, EventSource};
use amity_storage::calendar::{
    delete_calendar, fetch_calendar, insert_calendar, list_calendars, set_calendar_enabled,
    update_calendar_sync_state,
};
use amity_storage::connection::open_database;
use amity_storage::event::{prune_events_missing_from_feed, upsert_external_events};
use time::macros::datetime;

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-07-26 12:00:00 UTC)
}

#[tokio::test]
async fn insert_then_list_round_trips_with_default_sync_state() {
    let pool = db().await;
    let cal = CalendarBuilder::new("school", "https://example.test/s.ics")
        .category(CalendarCategory::School)
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    let all = list_calendars(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].calendar.name, "school");
    assert_eq!(all[0].sync.last_status, SyncStatus::Never);
    assert_eq!(all[0].sync.event_count, 0);
}

#[tokio::test]
async fn update_sync_state_persists() {
    let pool = db().await;
    let cal = CalendarBuilder::new("holidays", "https://example.test/h.ics")
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    let state = CalendarSyncState {
        last_synced_at: Some(now()),
        last_status: SyncStatus::Ok,
        last_error: None,
        event_count: 12,
    };
    update_calendar_sync_state(&pool, cal.id, &state)
        .await
        .unwrap();

    let fetched = fetch_calendar(&pool, cal.id).await.unwrap().unwrap();
    assert_eq!(fetched.sync.last_status, SyncStatus::Ok);
    assert_eq!(fetched.sync.event_count, 12);
}

#[tokio::test]
async fn upsert_external_events_inserts_then_updates() {
    let pool = db().await;
    // Two builds with the SAME source (calendar_id + uid) but different titles.
    let make = |title: &str| {
        EventBuilder::new()
            .title(title)
            .start_at(datetime!(2099-05-01 09:00:00 UTC))
            .source(EventSource::ics("uid-1", "cal-1", now()))
            .now(now())
            .build()
            .unwrap()
    };
    upsert_external_events(&pool, &[make("First")])
        .await
        .unwrap();
    upsert_external_events(&pool, &[make("Second")])
        .await
        .unwrap();

    // Exactly one row for that source, carrying the updated title.
    let events = amity_storage::event::list_events(&pool).await.unwrap();
    let mine: Vec<_> = events
        .iter()
        .filter(|e| e.source.external_id.as_deref() == Some("uid-1"))
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].title, "Second");
}

#[tokio::test]
async fn prune_removes_events_missing_from_the_feed() {
    let pool = db().await;
    let make = |uid: &str| {
        EventBuilder::new()
            .title(uid)
            .start_at(datetime!(2099-05-01 09:00:00 UTC))
            .source(EventSource::ics(uid, "cal-1", now()))
            .now(now())
            .build()
            .unwrap()
    };
    upsert_external_events(&pool, &[make("keep"), make("drop")])
        .await
        .unwrap();

    // Re-sync sees only "keep"; "drop" must be pruned.
    let deleted = prune_events_missing_from_feed(&pool, "cal-1", &["keep".to_owned()])
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let events = amity_storage::event::list_events(&pool).await.unwrap();
    assert!(
        events
            .iter()
            .all(|e| e.source.external_id.as_deref() != Some("drop"))
    );
}

#[tokio::test]
async fn set_enabled_and_delete_work() {
    let pool = db().await;
    let cal = CalendarBuilder::new("club", "https://example.test/c.ics")
        .now(now())
        .build()
        .unwrap();
    insert_calendar(&pool, &cal).await.unwrap();

    assert!(set_calendar_enabled(&pool, cal.id, false).await.unwrap());
    assert!(
        !fetch_calendar(&pool, cal.id)
            .await
            .unwrap()
            .unwrap()
            .calendar
            .enabled
    );

    assert!(delete_calendar(&pool, cal.id).await.unwrap());
    assert!(fetch_calendar(&pool, cal.id).await.unwrap().is_none());
}
