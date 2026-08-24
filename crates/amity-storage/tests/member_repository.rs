// member_repository.rs — integration tests for the member storage layer.
//
// Each test spins up a fresh in-memory SQLite database with all migrations
// applied and exercises the repository functions directly.
//
// What these tests verify:
//   • insert_member / list_member / fetch_member round-trip, including
//     optional initial and color (both None and Some).
//   • the MemberColor enum round-trips through storage.
//   • delete_member removes the row and reports whether one matched.
//   • list_members excludes the migration-0001 placeholder sentinel, while
//     fetch_member/delete_member remain able to target it directly.
//
// Note: migration 0001 seeds one placeholder member row (id
// 00000000-0000-7000-8000-000000000001) to satisfy legacy FK constraints, and
// migration 0006 backfills it rather than deleting it (see that migration's
// doc). `list_members` filters it out (see amity_storage::member's module
// doc), so these tests assert on the inserted row specifically rather than on
// exact list length — a real member's presence, not its position, is what
// matters — and a dedicated test below pins the exclusion itself.

use amity_core::member::{MemberBuilder, MemberColor};
use amity_storage::connection::open_database;
use amity_storage::member::{delete_member, fetch_member, insert_member, list_members};
use time::macros::datetime;

async fn db() -> sqlx::SqlitePool {
    open_database("sqlite::memory:").await.expect("db opens")
}

fn now() -> time::OffsetDateTime {
    datetime!(2026-08-24 10:00:00 UTC)
}

/// The migration-0001 placeholder sentinel's fixed id (mirrors the private
/// `PLACEHOLDER_MEMBER_ID` const in `amity_storage::member` — duplicated here
/// because that const is not, and should not be, public API).
fn placeholder_id() -> amity_core::ids::MemberId {
    "00000000-0000-7000-8000-000000000001"
        .parse()
        .expect("valid UUID literal")
}

#[tokio::test]
async fn insert_then_list_round_trips_with_initial_and_color() {
    let pool = db().await;
    let member = MemberBuilder::new("Alice")
        .initial("A")
        .color(MemberColor::Sage)
        .now(now())
        .build()
        .unwrap();
    insert_member(&pool, &member).await.unwrap();

    let all = list_members(&pool).await.unwrap();
    let alice = all
        .iter()
        .find(|m| m.id == member.id)
        .expect("inserted member present in list");
    assert_eq!(alice.display_name, "Alice");
    assert_eq!(alice.initial.as_deref(), Some("A"));
    assert_eq!(alice.color, Some(MemberColor::Sage));
    assert_eq!(alice.created_at, now());
    // The legacy placeholder must never appear in a listing.
    assert!(
        all.iter().all(|m| m.id != placeholder_id()),
        "placeholder sentinel must be excluded from list_members"
    );
}

#[tokio::test]
async fn insert_without_initial_or_color_round_trips_none() {
    let pool = db().await;
    let member = MemberBuilder::new("Ben").now(now()).build().unwrap();
    insert_member(&pool, &member).await.unwrap();

    let all = list_members(&pool).await.unwrap();
    let ben = all
        .iter()
        .find(|m| m.id == member.id)
        .expect("inserted member present in list");
    assert!(ben.initial.is_none());
    assert!(ben.color.is_none());
    // The legacy placeholder must never appear in a listing.
    assert!(
        all.iter().all(|m| m.id != placeholder_id()),
        "placeholder sentinel must be excluded from list_members"
    );
}

#[tokio::test]
async fn fetch_member_returns_none_for_unknown_id() {
    let pool = db().await;
    let unknown = amity_core::ids::MemberId::new();
    assert!(fetch_member(&pool, unknown).await.unwrap().is_none());
}

#[tokio::test]
async fn fetch_member_returns_matching_row() {
    let pool = db().await;
    let member = MemberBuilder::new("Cleo")
        .color(MemberColor::Ochre)
        .now(now())
        .build()
        .unwrap();
    insert_member(&pool, &member).await.unwrap();

    let fetched = fetch_member(&pool, member.id).await.unwrap().unwrap();
    assert_eq!(fetched.display_name, "Cleo");
    assert_eq!(fetched.color, Some(MemberColor::Ochre));
}

#[tokio::test]
async fn delete_member_removes_row_and_reports_match() {
    let pool = db().await;
    let member = MemberBuilder::new("Dan").now(now()).build().unwrap();
    insert_member(&pool, &member).await.unwrap();

    assert!(delete_member(&pool, member.id).await.unwrap());
    assert!(fetch_member(&pool, member.id).await.unwrap().is_none());

    assert!(!delete_member(&pool, member.id).await.unwrap());
}

#[tokio::test]
async fn list_members_excludes_placeholder_but_includes_real_members() {
    // Focused regression test for the sentinel-leak fix: a real member must
    // be listed, and the migration-0001 placeholder must not be, even though
    // both rows exist in the table.
    let pool = db().await;
    let member = MemberBuilder::new("Eve").now(now()).build().unwrap();
    insert_member(&pool, &member).await.unwrap();

    let all = list_members(&pool).await.unwrap();
    assert!(
        all.iter().any(|m| m.id == member.id),
        "real member must be present in list_members"
    );
    assert!(
        all.iter().all(|m| m.id != placeholder_id()),
        "placeholder sentinel must be absent from list_members"
    );
}

#[tokio::test]
async fn fetch_and_delete_member_remain_unfiltered_for_the_placeholder() {
    // list_members excludes the sentinel, but a direct lookup/delete by its
    // id must still behave predictably (unfiltered) — see the module doc.
    let pool = db().await;

    let fetched = fetch_member(&pool, placeholder_id()).await.unwrap();
    assert!(
        fetched.is_some(),
        "fetch_member must still find the placeholder row directly"
    );

    assert!(
        delete_member(&pool, placeholder_id()).await.unwrap(),
        "delete_member must still be able to remove the placeholder row directly"
    );
    assert!(
        fetch_member(&pool, placeholder_id())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn all_member_colors_round_trip_through_storage() {
    let pool = db().await;
    for color in [
        MemberColor::Sage,
        MemberColor::Clay,
        MemberColor::Ochre,
        MemberColor::Slate,
        MemberColor::Plum,
        MemberColor::Teal,
    ] {
        let member = MemberBuilder::new(format!("Member-{color}"))
            .color(color)
            .now(now())
            .build()
            .unwrap();
        insert_member(&pool, &member).await.unwrap();
        let fetched = fetch_member(&pool, member.id).await.unwrap().unwrap();
        assert_eq!(fetched.color, Some(color), "round-trip failed for {color}");
    }
}
