// calendar_sync.rs — integration tests for the sync job, with an injected fetch
// closure so no network is touched.

use amity_core::calendar::{CalendarBuilder, SyncStatus};
use amity_service::feeds::FetchError;
use amity_service::jobs::calendar_sync::run_once;
use amity_storage::calendar::{fetch_calendar, insert_calendar, list_calendars};
use amity_storage::connection::open_database;
use amity_storage::event::list_events;
use std::future::ready;
use time::macros::datetime;

const TWO: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20990501T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:B\r\nDTSTART:20990502T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const ONE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:A\r\nDTSTART:20990501T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn now() -> time::OffsetDateTime {
    datetime!(2099-04-27 12:00:00 UTC)
}

async fn seeded_pool() -> (sqlx::SqlitePool, amity_core::ids::CalendarId) {
    let pool = open_database("sqlite::memory:").await.unwrap();
    let cal = CalendarBuilder::new("feed", "https://example.test/f.ics")
        .now(now())
        .build()
        .unwrap();
    let id = cal.id;
    insert_calendar(&pool, &cal).await.unwrap();
    (pool, id)
}

#[tokio::test]
async fn sync_ingests_then_prunes_and_records_ok() {
    let (pool, _id) = seeded_pool().await;

    run_once(&pool, now(), |_url| {
        ready(Ok::<_, FetchError>(TWO.to_owned()))
    })
    .await
    .unwrap();
    assert_eq!(list_events(&pool).await.unwrap().len(), 2);

    // Re-sync with B removed from the feed.
    run_once(&pool, now(), |_url| {
        ready(Ok::<_, FetchError>(ONE.to_owned()))
    })
    .await
    .unwrap();
    let events = list_events(&pool).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source.external_id.as_deref(), Some("a"));

    let cal = &list_calendars(&pool).await.unwrap()[0];
    assert_eq!(cal.sync.last_status, SyncStatus::Ok);
    assert_eq!(cal.sync.event_count, 1);
}

#[tokio::test]
async fn a_failed_fetch_records_unreachable_and_keeps_events() {
    let (pool, id) = seeded_pool().await;
    run_once(&pool, now(), |_url| {
        ready(Ok::<_, FetchError>(TWO.to_owned()))
    })
    .await
    .unwrap();

    // Next sync fails to fetch.
    run_once(&pool, now(), |_url| {
        ready(Err::<String, _>(FetchError::BadStatus(503)))
    })
    .await
    .unwrap();

    // Events are still there; status reflects the failure.
    assert_eq!(list_events(&pool).await.unwrap().len(), 2);
    let cal = fetch_calendar(&pool, id).await.unwrap().unwrap();
    assert_eq!(cal.sync.last_status, SyncStatus::Unreachable);
}
