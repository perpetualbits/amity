// inbox_repository.rs — integration tests for the inbox storage layer.
//
// Each test spins up a fresh in-memory SQLite database, applies all migrations,
// and exercises the three repository functions. The in-memory database is
// isolated per test — no shared state, no ordering dependencies between tests
// even when the test runner executes them in parallel.
//
// What these tests verify:
//   • The round-trip invariant: values written via insert_inbox_item must come
//     back unchanged via fetch_inbox_item (every field, not just id).
//   • The "not found" contract: fetching a non-existent ID returns None, not
//     an error.
//   • List ordering: list_recent_inbox_items returns items newest-first.
//   • List limiting: the `limit` parameter caps the result count.
//   • Enum serialisation: all InboxSource variants survive a write→read cycle
//     without the storage layer choking on an unknown string.
//   • Triaged item: the triaged_to field is preserved when non-null.
//
// These tests do not test business logic (that lives in amity-core unit tests)
// or HTTP semantics (that lives in amity-service tests).

use amity_core::ids::{InboxItemId, MemberId};
use amity_core::inbox::{InboxItem, InboxItemBuilder, InboxSource, TriageState, TypedEntityRef};
use amity_storage::connection::open_database;
use amity_storage::inbox::{fetch_inbox_item, insert_inbox_item, list_recent_inbox_items};
// The `datetime!` macro produces a compile-time `OffsetDateTime` from a literal,
// which gives tests deterministic timestamps without system-clock calls.
use time::macros::datetime;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Open a fresh in-memory database with all migrations applied.
///
/// Each test calls this independently so there is no shared mutable state
/// between tests, even when the test runner runs them in parallel.
///
/// The `:memory:` URL creates a new, empty `SQLite` database every time.
/// `open_database` applies all migrations before returning the pool.
async fn open_test_db() -> sqlx::SqlitePool {
    // `:memory:` creates a fresh, empty database — completely isolated from other calls.
    open_database("sqlite::memory:")
        .await
        .expect("in-memory database should always open")
}

/// The placeholder member UUID inserted by migration 0001.
///
/// Tests use this UUID when `captured_by` is needed, because the database
/// has a foreign-key constraint on `members(id)` and the in-memory database
/// starts with only this one member row (inserted by the migration itself).
fn placeholder_member_id() -> MemberId {
    // Same UUID as the one in 0001_initial.sql — must match exactly or the FK fails.
    // Construct the MemberId from the UUID string.
    MemberId(
        uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
            .expect("hardcoded UUID is valid"),
    )
}

/// Build a minimal valid inbox item with a specific `captured_at` timestamp.
///
/// `text` and `captured_at` are parameterised so tests can create items with
/// predictable ordering and distinguishable content without duplicating the
/// builder call.
fn make_item(text: &str, captured_at: time::OffsetDateTime) -> InboxItem {
    // Build a minimal but valid item using the builder chain.
    InboxItemBuilder::new()
        // Set the raw text as the verbatim captured thought.
        .raw_text(text)
        // captured_by uses the placeholder member inserted by migration 0001.
        .captured_by(placeholder_member_id())
        // now sets the `captured_at` timestamp for ordering tests.
        .now(captured_at)
        .build()
        // `expect` is acceptable in test helpers — a panic here means
        // the test setup data is invalid, not a production invariant violation.
        .expect("test item should be valid")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn insert_and_fetch_round_trip() {
    // An item written to the database should come back with all fields intact.
    // If any field silently changes (e.g. a timezone gets stripped, an enum
    // serialises differently), this test catches it.
    // Each test gets its own isolated database — no shared mutable state.
    let pool = open_test_db().await;
    // Build the item we'll write and later verify field-by-field.
    let item = make_item("pick up pencils", datetime!(2026-05-25 10:00:00 UTC));

    // Write the item to the database.
    insert_inbox_item(&pool, &item)
        .await
        .expect("insert should succeed");

    // Read it back by ID to verify the round-trip.
    let fetched = fetch_inbox_item(&pool, item.id)
        .await
        .expect("fetch should succeed")
        // `expect` on the Option: if the item isn't found immediately after
        // insert, something is very wrong with the write or the fetch query.
        .expect("item should exist after insert");

    // Assert each field individually so test failures name the differing field.
    assert_eq!(fetched.id, item.id);
    // raw_text must be verbatim — no normalisation should occur during storage.
    assert_eq!(fetched.raw_text, item.raw_text);
    // captured_by: UUID round-trip through TEXT must be lossless.
    assert_eq!(fetched.captured_by, item.captured_by);
    // OffsetDateTime comparison: the stored/parsed value should equal the
    // original. RFC 3339 round-trip preserves UTC offset exactly. If this
    // fails, the timestamp is either being truncated or the timezone offset
    // is being dropped during serialisation.
    assert_eq!(fetched.captured_at, item.captured_at);
    // source: enum string round-trip through TEXT.
    assert_eq!(fetched.source, item.source);
    // triage_state: enum string round-trip through TEXT.
    assert_eq!(fetched.triage_state, item.triage_state);
    // triaged_to: None round-trip (new items start untriaged).
    assert_eq!(fetched.triaged_to, item.triaged_to);
}

#[tokio::test]
async fn fetch_nonexistent_id_returns_none() {
    // "Not found" must return None, not an error. The service layer decides
    // what to do with a missing item (usually a 404 response). If the storage
    // layer returned an error, the service would have to inspect the error to
    // decide whether it's "not found" vs "I/O failure", which is fragile.
    // Fresh database — nothing has been inserted, so any ID is missing.
    let pool = open_test_db().await;
    // Generate a fresh ID that was never inserted into this database.
    let missing_id = InboxItemId::new();

    // The fetch must succeed (no error) and return None (not found).
    let result = fetch_inbox_item(&pool, missing_id)
        .await
        .expect("fetch should not error for a missing ID");

    // None is the correct representation of "item does not exist".
    assert!(result.is_none(), "expected None for a non-existent ID");
}

#[tokio::test]
async fn list_recent_returns_items_newest_first() {
    // Items should be returned in descending captured_at order regardless of
    // insertion order. The recent-list query uses ORDER BY captured_at DESC.
    // This test inserts the older item first to verify ordering isn't just
    // "insertion order" (which would happen to work but isn't the contract).
    let pool = open_test_db().await;

    // Older item: 08:00
    let older = make_item("older item", datetime!(2026-05-25 08:00:00 UTC));
    // Newer item: 10:00 — two hours later
    let newer = make_item("newer item", datetime!(2026-05-25 10:00:00 UTC));

    // Insert older first, then newer — so if the query returns insertion order
    // instead of timestamp order, the test would fail in the expected way.
    insert_inbox_item(&pool, &older)
        .await
        .expect("insert older");
    // Insert newer second — if ordering relied on insert order, this would place
    // newer last; the ORDER BY captured_at DESC should put it first instead.
    insert_inbox_item(&pool, &newer)
        .await
        .expect("insert newer");

    // Retrieve both items; limit=20 ensures both are included.
    let items = list_recent_inbox_items(&pool, 20)
        .await
        .expect("list should succeed");

    // Should be exactly 2 items — no more, no fewer.
    assert_eq!(items.len(), 2, "expected exactly two items");
    // The first item must be the newer one (highest captured_at).
    assert_eq!(items[0].raw_text, "newer item", "first item must be newest");
    // The second item must be the older one (lowest captured_at).
    assert_eq!(
        items[1].raw_text, "older item",
        "second item must be oldest"
    );
}

#[tokio::test]
async fn list_recent_respects_limit() {
    // The `limit` parameter must cap the number of returned items, even when
    // more items exist. The service enforces a maximum of 100; the storage
    // layer just applies whatever limit it receives from the caller.
    // Isolated database — exactly three items will be inserted.
    let pool = open_test_db().await;

    // Insert three items with distinct timestamps for deterministic ordering.
    for i in 0u8..3 {
        // Stagger timestamps by 1 minute each to ensure distinct ordering.
        let ts = datetime!(2026-05-25 10:00:00 UTC) + time::Duration::seconds(i64::from(i) * 60);
        // Build item N with timestamp N minutes after the base time.
        let item = make_item(&format!("item {i}"), ts);
        // Insert each item into the database.
        insert_inbox_item(&pool, &item).await.expect("insert");
    }

    // Request only 2, even though 3 exist — the limit must be respected.
    let items = list_recent_inbox_items(&pool, 2)
        .await
        // A failure here is a storage-layer bug, not a limit violation.
        .expect("list should succeed");

    // The storage layer must honour the limit — returning all rows regardless
    // of the limit would be a data leak in some contexts.
    // Result must be exactly 2, not 0 or 3.
    assert_eq!(items.len(), 2, "limit should cap at 2");
}

#[tokio::test]
async fn list_recent_with_empty_database_returns_empty_vec() {
    // An empty inbox must return an empty Vec, not an error. This is the
    // expected state on first run and the correct representation of "no data".
    // Empty database — no items have been inserted.
    let pool = open_test_db().await;

    // Query an empty database; the result must be Ok([]), not Err.
    let items = list_recent_inbox_items(&pool, 20)
        .await
        .expect("list should succeed on empty db");

    // An empty Vec is the correct response for "inbox exists but has no items".
    // An Err result or a non-empty Vec would both indicate storage bugs.
    // `is_empty()` is cleaner than checking `len() == 0`.
    assert!(items.is_empty(), "expected empty vec for empty database");
}

#[tokio::test]
async fn all_inbox_sources_round_trip_through_storage() {
    // Every InboxSource variant must survive a write→read cycle. If the TEXT
    // representation in `InboxSource::to_string` doesn't match the one in
    // `InboxSource::from_str`, the storage layer will fail to parse the row.
    // One database for all source variants in this test.
    let pool = open_test_db().await;
    // Base time for all items; each item gets a unique offset to avoid ordering ties.
    let base_time = datetime!(2026-05-25 10:00:00 UTC);

    // Enumerate every known InboxSource variant so coverage is complete.
    let sources = [
        InboxSource::Touch,
        InboxSource::Mobile,
        InboxSource::Share,
        InboxSource::ForwardEmail,
        InboxSource::Voice,
    ];

    // Write and read each variant, verifying the round-trip for each one.
    for (i, source) in sources.iter().enumerate() {
        // Stagger timestamps by 1 minute so each item has a unique captured_at.
        // Without this, items could be returned in arbitrary order by the list query.
        // `i` is bounded by the sources array length (5); the cast to i64
        // cannot overflow. The lint is suppressed because the range is known.
        #[allow(clippy::cast_possible_wrap)]
        // Offset this item's timestamp to keep items in a deterministic order.
        let ts = base_time + time::Duration::seconds(i as i64 * 60);
        // Build an item with this specific source variant.
        let item = InboxItemBuilder::new()
            .raw_text(format!("source test {i}"))
            .captured_by(placeholder_member_id())
            .now(ts)
            .source(*source)
            .build()
            // `unwrap` is acceptable in tests — a failure here means broken test setup.
            .unwrap();

        // Persist the item with this source variant.
        insert_inbox_item(&pool, &item).await.expect("insert");

        // Retrieve the item and verify the source field survived the round-trip.
        let fetched = fetch_inbox_item(&pool, item.id)
            .await
            .expect("fetch")
            .expect("exists");

        // If the Display↔FromStr contract is broken for this variant, this fails.
        assert_eq!(
            fetched.source, *source,
            "source round-trip failed for {source:?}"
        );
    }
}

#[tokio::test]
async fn triaged_item_stores_typed_entity_ref() {
    // When an item has been triaged (triage_state = Typed, triaged_to = Some(...)),
    // the reference must round-trip through storage intact. If `triaged_to` is
    // mistakenly stored as NULL or truncated, the triage link is lost.
    // Fresh database for this test.
    let pool = open_test_db().await;
    // Fixed timestamp — the exact value is not significant for this test.
    let ts = datetime!(2026-05-25 10:00:00 UTC);

    // Build a base item, then override the triage fields. The builder always
    // sets Untriaged; actual triage transitions happen in the service layer
    // after capture, so we simulate that here by mutating after construction.
    // Mutate after build to simulate the triage transition from the service layer.
    let mut item = make_item("buy printer ink", ts);
    // The item has been triaged to a Task entity — set both state and reference.
    item.triage_state = TriageState::Typed;
    // The triaged_to reference must round-trip intact through storage.
    item.triaged_to = Some(TypedEntityRef::new(
        "task",
        uuid::Uuid::parse_str("018f1a2b-0000-7000-8000-000000000002").unwrap(),
    ));

    // Persist the triaged item.
    insert_inbox_item(&pool, &item).await.expect("insert");

    // Read it back and verify both triage fields are present and unchanged.
    let fetched = fetch_inbox_item(&pool, item.id)
        .await
        .expect("fetch")
        .expect("exists");

    // Both the state and the reference must round-trip. A triage_state of
    // Typed with a None triaged_to would be an inconsistent state in storage.
    assert_eq!(fetched.triage_state, TriageState::Typed);
    // triaged_to must also round-trip; a None value would lose the triage link.
    assert_eq!(fetched.triaged_to, item.triaged_to);
}
