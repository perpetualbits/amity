// feeds.rs — outbound HTTP fetch of an external ICS feed.
//
// This is Amity's FIRST outbound network egress (ADR-0004 territory): every
// prior task talks only to the local SQLite database. A subscribed calendar
// (school, club, waste, holiday, personal — brief §7) supplies an arbitrary
// URL the household typed or pasted in, so this module treats every fetch as
// hitting an untrusted third party and applies three independent guards:
//
//   1. A bounded client timeout, so one slow/hung server cannot stall the
//      6-hourly sync job indefinitely.
//   2. A bounded redirect chain, so a feed cannot bounce the request through
//      an unbounded (or malicious) redirect loop.
//   3. A hard cap on response size, enforced while streaming (not after a
//      full download), so a feed that serves gigabytes of data cannot exhaust
//      memory or disk.
//
// `fetch` performs NO parsing — it only returns the raw body text. Parsing
// lives in `amity_core::ics`, which stays network-free and independently
// testable. Keeping the split this way means the only code path that ever
// touches the network is this one small function.

// The error type this module returns; `thiserror` derives `std::error::Error`.
use thiserror::Error;

/// The hard cap on a fetched feed's body size, in bytes (5 MiB).
///
/// Real ICS feeds — even a busy family's combined school/club/waste calendar
/// — are a few hundred KiB at most. 5 MiB gives generous headroom while still
/// bounding worst-case memory use for a feed that misbehaves (or is hostile).
const MAX_FEED_BYTES: usize = 5 * 1024 * 1024;

/// How long the whole request (connect + read the whole body) may take, in
/// seconds. A feed host that is slow or has silently dropped the connection
/// must not be allowed to block the sync job's 6-hour cycle indefinitely.
const REQUEST_TIMEOUT_SECS: u64 = 20;

/// How many redirect hops `fetch` will follow before giving up.
///
/// A normal feed redirect (e.g. a calendar host moving to a CDN) is at most
/// one or two hops; five is generous headroom without leaving the door open
/// to an unbounded (or looping) redirect chain.
const MAX_REDIRECTS: usize = 5;

/// Failure modes for [`fetch`]. Every variant maps to one of the three egress
/// guards in the module docs, plus the generic network/timeout case.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The underlying HTTP client failed — DNS, connect, TLS, timeout, or a
    /// redirect-limit violation. `reqwest`'s own message is preserved as-is
    /// so operators see the real cause without a lossy re-wrap.
    #[error("request failed: {0}")]
    Request(String),

    /// The response body exceeded [`MAX_FEED_BYTES`] while streaming. The
    /// partial body already read is discarded — a truncated ICS payload
    /// would parse into a corrupt/incomplete calendar, which is worse than
    /// treating the whole fetch as failed.
    #[error("response exceeded {} bytes", MAX_FEED_BYTES)]
    TooLarge,

    /// The server responded with a non-2xx status. The numeric code is kept
    /// so `Unreachable` diagnostics in the sync job can show it verbatim.
    #[error("unexpected status: {0}")]
    BadStatus(u16),
}

/// Fetch an ICS feed's raw body text from `url`.
///
/// Builds a fresh `reqwest::Client` per call (feeds are synced at most once
/// per interval per calendar — a handful of times an hour across the whole
/// household — so the connection-pooling benefit of a shared client is not
/// worth the complexity of threading one through the job). Applies the three
/// egress guards documented at the top of this module; see each `FetchError`
/// variant for which guard produced it.
///
/// # Errors
///
/// Returns [`FetchError::Request`] for a network/timeout/redirect-limit
/// failure, [`FetchError::BadStatus`] for a non-2xx response, or
/// [`FetchError::TooLarge`] if the body exceeds [`MAX_FEED_BYTES`].
pub async fn fetch(url: String) -> Result<String, FetchError> {
    // Guard 1: bounded timeout + Guard 2: bounded redirect chain, both set on
    // the client so they apply to the whole request/response cycle,
    // including any redirect hops.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        // Client construction only fails on a broken TLS backend config,
        // which is a build-time invariant, not a per-request condition —
        // still surfaced as a `Request` error rather than a panic.
        .map_err(|e| FetchError::Request(e.to_string()))?;

    // Issue the GET; DNS failure, connection refusal, TLS failure, or the
    // client-level timeout all land here as `reqwest::Error`. `mut` because
    // the streaming loop below pulls chunks out of it one at a time.
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Request(e.to_string()))?;

    // Non-2xx is rejected before touching the body at all — no point
    // streaming a 404/500 error page through the size guard below.
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::BadStatus(status.as_u16()));
    }

    // Guard 3: stream the body and abort as soon as the running total would
    // exceed the cap, rather than buffering the whole response first. This
    // bounds peak memory use to roughly one chunk over the cap, not the
    // feed's full (possibly unbounded) size.
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| FetchError::Request(e.to_string()))?
    {
        // Check BEFORE extending so a feed that is exactly one huge chunk
        // still aborts promptly rather than allocating the whole thing.
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    // The accumulated bytes must be valid UTF-8 for an ICS feed (RFC 5545
    // text). A non-UTF-8 body is exceedingly rare in practice and is treated
    // the same as any other malformed response at the network layer.
    String::from_utf8(body).map_err(|e| FetchError::Request(e.to_string()))
}
