// feeds_egress.rs — HTTP-level tests for feeds::fetch's egress guards.
//
// The unit/integration tests elsewhere drive the sync job through an injected
// fetch closure, so the real network path in feeds::fetch was previously only
// reasoned about, never exercised. These tests stand up a real local HTTP
// server (wiremock) and drive fetch against it to prove each egress guard
// actually fires:
//   • a non-2xx status becomes FetchError::BadStatus(code);
//   • a body over the 5 MiB cap becomes FetchError::TooLarge (and a body
//     exactly at the cap still succeeds — the cap is inclusive);
//   • an unbounded redirect chain is refused via the bounded redirect policy.
//
// The 20s client timeout is deliberately NOT tested here: exercising it would
// require a mock that stalls for >20s, making the suite slow for little gain.
// It is a single client-builder setting verified by inspection.

use amity_service::feeds::{FetchError, fetch};
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Mirrors the private `MAX_FEED_BYTES` in feeds.rs (5 MiB). Kept in sync by
// hand; if the source constant changes, the boundary tests below must follow.
const FEED_CAP_BYTES: usize = 5 * 1024 * 1024;

// A minimal valid ICS body for the happy-path assertion.
const SMALL_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Hi\r\nDTSTART:20990101T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[tokio::test]
async fn a_2xx_response_returns_the_raw_body() {
    // A well-behaved feed: 200 with an ICS body.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SMALL_ICS))
        .mount(&server)
        .await;

    let body = fetch(format!("{}/feed.ics", server.uri()))
        .await
        .expect("a 200 response yields the body");
    // fetch returns the body verbatim — it performs no parsing of its own.
    assert_eq!(body, SMALL_ICS);
}

#[tokio::test]
async fn a_non_2xx_status_maps_to_bad_status() {
    // The guard rejects any non-2xx before reading the body at all.
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream is down"))
        .mount(&server)
        .await;

    let err = fetch(format!("{}/feed.ics", server.uri()))
        .await
        .expect_err("a 503 is an error, not a body");
    // The numeric code is preserved so the sync job can surface it.
    assert!(
        matches!(err, FetchError::BadStatus(503)),
        "expected BadStatus(503), got {err:?}"
    );
}

#[tokio::test]
async fn a_body_over_the_cap_is_refused_as_too_large() {
    // One byte past the cap must trip the streaming size guard.
    let server = MockServer::start().await;
    let oversize = "A".repeat(FEED_CAP_BYTES + 1);
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string(oversize))
        .mount(&server)
        .await;

    let err = fetch(format!("{}/feed.ics", server.uri()))
        .await
        .expect_err("an oversize body is refused");
    // The guard counts actual streamed bytes, not the Content-Length header.
    assert!(
        matches!(err, FetchError::TooLarge),
        "expected TooLarge, got {err:?}"
    );
}

#[tokio::test]
async fn a_body_exactly_at_the_cap_still_succeeds() {
    // The cap is inclusive: exactly MAX_FEED_BYTES must pass, proving the
    // guard uses `>` and not `>=` (a regression here would reject legitimate
    // large-but-in-bounds feeds).
    let server = MockServer::start().await;
    let at_cap = "A".repeat(FEED_CAP_BYTES);
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string(at_cap.clone()))
        .mount(&server)
        .await;

    let body = fetch(format!("{}/feed.ics", server.uri()))
        .await
        .expect("a body exactly at the cap is within bounds");
    assert_eq!(body.len(), FEED_CAP_BYTES);
    assert_eq!(body, at_cap);
}

#[tokio::test]
async fn an_unbounded_redirect_chain_is_refused() {
    // Every request 302s back to the server, an endless loop. The bounded
    // redirect policy must give up (rather than follow forever) and surface a
    // Request error once the hop limit is exceeded.
    let server = MockServer::start().await;
    let loop_target = format!("{}/loop", server.uri());
    Mock::given(any())
        .respond_with(ResponseTemplate::new(302).insert_header("Location", loop_target.as_str()))
        .mount(&server)
        .await;

    let err = fetch(format!("{}/start", server.uri()))
        .await
        .expect_err("an endless redirect chain must not be followed forever");
    // A redirect-limit violation surfaces as a Request error (reqwest's own
    // "too many redirects" message is preserved inside it).
    assert!(
        matches!(err, FetchError::Request(_)),
        "expected Request(_), got {err:?}"
    );
}
