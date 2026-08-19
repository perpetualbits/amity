// week_api.rs — integration tests for `GET /api/v1/week`.
//
// Mirrors the harness in calendar_api.rs/event_api.rs: build the full axum app
// over a fresh in-memory database and drive real HTTP requests. The fixed
// week under test is 2099-05-04 (Monday) .. 2099-05-10 (Sunday), seeded with
// one scenario per Slice 2 requirement: an external recurring event with an
// EXDATE, a rescheduled event, an annotated event, an all-day event, and a
// dated task (one open, one completed).

use amity_service::build_app;
use amity_service::feeds::FetchError;
use amity_service::jobs::calendar_sync::run_once;
use amity_storage::connection::open_database;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::future::ready;
use tower::ServiceExt;

// ─── Harness (mirrors calendar_api.rs / event_api.rs) ───────────────────────

/// A fresh app over an isolated in-memory database. Cloning the returned Router
/// shares the same pool, so a create and a later query see each other's writes.
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

/// Read a response body as JSON.
async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body readable");
    serde_json::from_slice(&bytes).expect("valid JSON")
}

// ─── Fixtures ─────────────────────────────────────────────────────────────

/// A twice-weekly (Tuesday + Friday) external event starting 2099-04-21, with
/// an EXDATE removing the 2099-05-08 (Friday) occurrence — the one that would
/// otherwise land inside the test week. The 2099-05-05 (Tuesday) occurrence,
/// in the same week, is untouched, so the test can prove both "an in-window
/// occurrence surfaces" and "the EXDATE'd one does not" from one fixture.
const RECURRING_WITH_EXDATE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:club-twice-weekly\r\nSUMMARY:weekly recurring\r\nDTSTART:20990421T090000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=TU,FR\r\nEXDATE:20990508T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Seed the fixed week (2099-05-04 Monday .. 2099-05-10 Sunday) with one
/// scenario per Slice 2 requirement. Returns the shared pool/app pair so the
/// caller can drive further requests against the same database.
async fn seed_week() -> axum::Router {
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should open");
    let app = build_app(db.clone());

    // ── External recurring event with an EXDATE ────────────────────────────
    let create_cal = post_json(
        app.clone(),
        "/api/v1/calendars",
        json!({ "name": "club", "url": "https://example.test/club.ics", "category": "club" }),
    )
    .await;
    assert_eq!(create_cal.status(), StatusCode::CREATED);
    let now = time::macros::datetime!(2099-04-20 12:00:00 UTC);
    let report = run_once(&db, now, |_url| {
        ready(Ok::<_, FetchError>(RECURRING_WITH_EXDATE.to_owned()))
    })
    .await
    .expect("sync run succeeds");
    assert_eq!(report.calendars_synced, 1);
    assert_eq!(report.events_upserted, 1);

    // ── All-day event — Monday ──────────────────────────────────────────────
    let all_day = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "king's day", "start_at": "2099-05-04T00:00:00+00:00", "all_day": true }),
    )
    .await;
    assert_eq!(all_day.status(), StatusCode::CREATED);

    // ── Rescheduled event — Wednesday, moved from 07:00 to 09:00 ────────────
    let bin_day = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "bin day", "start_at": "2099-05-06T07:00:00+00:00" }),
    )
    .await;
    assert_eq!(bin_day.status(), StatusCode::CREATED);
    let bin_day_id = body_json(bin_day).await["id"]
        .as_str()
        .expect("event id")
        .to_owned();
    let reschedule = post_json(
        app.clone(),
        &format!("/api/v1/events/{bin_day_id}/override"),
        json!({
            "instance_date": "2099-05-06",
            "action": "reschedule",
            "payload": "2099-05-06T09:00:00+00:00"
        }),
    )
    .await;
    assert_eq!(reschedule.status(), StatusCode::CREATED);

    // ── Annotated event — Thursday ──────────────────────────────────────────
    let school_trip = post_json(
        app.clone(),
        "/api/v1/events",
        json!({ "title": "school trip", "start_at": "2099-05-07T08:00:00+00:00" }),
    )
    .await;
    assert_eq!(school_trip.status(), StatusCode::CREATED);
    let school_trip_id = body_json(school_trip).await["id"]
        .as_str()
        .expect("event id")
        .to_owned();
    let annotate = post_json(
        app.clone(),
        &format!("/api/v1/events/{school_trip_id}/override"),
        json!({
            "instance_date": "2099-05-07",
            "action": "annotate",
            "payload": "bring wellies"
        }),
    )
    .await;
    assert_eq!(annotate.status(), StatusCode::CREATED);

    // ── Dated task, open — Wednesday ────────────────────────────────────────
    let open_task = post_json(
        app.clone(),
        "/api/v1/tasks",
        json!({ "title": "water plants", "due_by": "2099-05-06T15:00:00+00:00" }),
    )
    .await;
    assert_eq!(open_task.status(), StatusCode::CREATED);

    // ── Dated task, completed — Saturday, must not surface ──────────────────
    let done_task = post_json(
        app.clone(),
        "/api/v1/tasks",
        json!({ "title": "chore already done", "due_by": "2099-05-09T10:00:00+00:00" }),
    )
    .await;
    assert_eq!(done_task.status(), StatusCode::CREATED);
    let done_task_id = body_json(done_task).await["id"]
        .as_str()
        .expect("task id")
        .to_owned();
    let complete = post_json(
        app.clone(),
        &format!("/api/v1/tasks/{done_task_id}/complete"),
        json!({ "instance_date": "2099-05-09" }),
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);

    app
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn week_returns_seven_monday_start_days_with_correct_dates() {
    let app = seed_week().await;

    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["start"], "2099-05-04");
    let days = body["days"].as_array().expect("days array");
    assert_eq!(days.len(), 7);
    let expected_dates = [
        "2099-05-04",
        "2099-05-05",
        "2099-05-06",
        "2099-05-07",
        "2099-05-08",
        "2099-05-09",
        "2099-05-10",
    ];
    for (day, expected) in days.iter().zip(expected_dates) {
        assert_eq!(day["date"], expected);
    }
}

#[tokio::test]
async fn week_returns_the_monday_of_the_queried_dates_week_even_mid_week() {
    // Querying a Wednesday inside the week must resolve to the same Monday.
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-06").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["start"], "2099-05-04");
}

#[tokio::test]
async fn recurring_external_event_lands_on_its_occurrence_and_the_exdated_one_is_absent() {
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");

    // Tuesday (index 1) — the untouched occurrence surfaces.
    let tuesday_items = days[1]["items"].as_array().expect("items array");
    assert!(
        tuesday_items
            .iter()
            .any(|i| i["title"] == "weekly recurring"),
        "the Tuesday occurrence must surface"
    );

    // Friday (index 4) — the EXDATE'd occurrence must be absent.
    let friday_items = days[4]["items"].as_array().expect("items array");
    assert!(
        !friday_items
            .iter()
            .any(|i| i["title"] == "weekly recurring"),
        "the EXDATE'd Friday occurrence must not surface"
    );
}

#[tokio::test]
async fn all_day_event_leads_its_day() {
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");

    // Monday (index 0) — the all-day banner is the only item and leads.
    let monday_items = days[0]["items"].as_array().expect("items array");
    assert_eq!(monday_items.len(), 1);
    assert_eq!(monday_items[0]["title"], "king's day");
    assert_eq!(monday_items[0]["all_day"], true);
}

#[tokio::test]
async fn rescheduled_event_surfaces_at_its_new_time_flagged_rescheduled() {
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");

    // Wednesday (index 2) — "bin day" (rescheduled) and "water plants" (task).
    // Events sort ahead of tasks, so the event is first.
    let wed_items = days[2]["items"].as_array().expect("items array");
    assert_eq!(wed_items.len(), 2, "bin day event + water plants task");
    assert_eq!(wed_items[0]["title"], "bin day");
    assert_eq!(wed_items[0]["at"], "2099-05-06T09:00:00Z");
    assert_eq!(wed_items[0]["rescheduled"], true);
    assert_eq!(wed_items[1]["title"], "water plants");
    assert_eq!(wed_items[1]["kind"], "task");
}

#[tokio::test]
async fn annotated_event_carries_its_note_with_timing_unchanged() {
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");

    // Thursday (index 3) — the annotated event, alone.
    let thu_items = days[3]["items"].as_array().expect("items array");
    assert_eq!(thu_items.len(), 1);
    assert_eq!(thu_items[0]["title"], "school trip");
    assert_eq!(thu_items[0]["annotation"], "bring wellies");
    assert_eq!(thu_items[0]["at"], "2099-05-07T08:00:00Z");
    assert_eq!(thu_items[0]["rescheduled"], false);
}

#[tokio::test]
async fn open_dated_task_appears_and_a_completed_one_does_not() {
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");

    // Wednesday — the open task is present (checked alongside the event above
    // in the reschedule test too, but assert independently here).
    let wed_items = days[2]["items"].as_array().expect("items array");
    assert!(wed_items.iter().any(|i| i["title"] == "water plants"));

    // Saturday (index 5) — the completed task must NOT surface.
    let sat_items = days[5]["items"].as_array().expect("items array");
    assert!(
        !sat_items.iter().any(|i| i["title"] == "chore already done"),
        "a done task must never surface on Week"
    );
    assert!(sat_items.is_empty(), "Saturday has no other seeded items");
}

#[tokio::test]
async fn sunday_and_friday_are_otherwise_empty() {
    // A quiet sanity check that nothing leaked onto days with no scenario.
    let app = seed_week().await;
    let resp = get(app, "/api/v1/week?start=2099-05-04").await;
    let body = body_json(resp).await;
    let days = body["days"].as_array().expect("days array");
    assert!(days[6]["items"].as_array().expect("items array").is_empty());
}

#[tokio::test]
async fn week_defaults_to_the_current_week_when_start_is_absent() {
    let app = build_test_app().await;

    // Compute the Monday of "now"'s week the same way the handler must.
    let now = time::OffsetDateTime::now_utc();
    let today = now.date();
    let offset = today.weekday().number_days_from_monday();
    let expected_monday = today - time::Duration::days(i64::from(offset));
    let fmt = time::format_description::parse("[year]-[month]-[day]").unwrap();
    let expected = expected_monday.format(&fmt).unwrap();

    let resp = get(app, "/api/v1/week").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["start"], expected);
    assert_eq!(body["days"].as_array().expect("days array").len(), 7);
}

#[tokio::test]
async fn malformed_start_query_is_400() {
    let app = build_test_app().await;
    let resp = get(app, "/api/v1/week?start=not-a-date").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
