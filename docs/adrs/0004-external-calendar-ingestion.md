# ADR-0004 — External calendar ingestion: read-only ICS, egress guards, plaintext feed URLs

**Date:** 2026-08-18
**Status:** Accepted

---

## Context

Task 4 shipped the hub-native half of the calendar: an `Event` entity, an
`EventOverride` overlay, storage, an API, and surfacing onto Today. It
deliberately deferred the other half of brief §7's calendar **aggregator**
posture — Amity is meant to gather a household's external calendars (school
per child, sports/club feeds, the municipal afvalkalender, the public NL
holiday calendar, adult members' personal Google/Apple calendars) and show
them on Today alongside native tasks and events, without ever becoming a
second source of truth for any of them. Task 5 fills that half.

Several design questions had to be settled:

1. **How does Amity read an external calendar — OAuth, or a plain feed URL?**
2. **This is the system's first outbound network call.** Every prior task —
   inbox, tasks, recurrence, events — talks only to the local SQLite database
   over a loopback-only service (brief §2's categorical "no surveillance
   vectors, no commercial data flow" commitment, and a claim the project map
   has made since Task 1). What guards does opening a socket for the first
   time need, and how do we keep the "no data leaves the device" claim
   honest rather than silently broken?
3. **Where and how are feed URLs stored?** A personal Google/Apple calendar's
   ICS URL embeds a long-lived secret token — anyone with the URL can read
   the calendar. Does this change how the URL is stored?
4. **How should external RRULEs be expanded?** The native recurrence engine
   (ADR-0002) validates a deliberately restricted RRULE subset. Real-world
   calendar feeds are not obliged to respect that subset.

---

## Decision

### Aggregator posture: read-only ICS, not OAuth

Amity subscribes to external calendars by their ICS feed URL — `https://`,
or `webcal://` (rewritten to `https://` at construction; see below) — and
never writes back. `CalendarCategory` covers the household's real sources
(school, club, waste, holiday, personal) and the hub-native calendar from
Task 4 remains the only calendar Amity can write to. `EventSourceKind::Ics`
marks every event pulled from a feed as `read_only`, a flag Task 4 already
laid down for this task to fill.

**Why not OAuth to Google/Apple?** OAuth would let Amity create, edit, and
delete events on a household member's real calendar — a capability the
product does not want. It also multiplies the integration surface (two
vendor APIs, token refresh, revocation handling, scope creep) for a feature
whose whole value is "show me what's already on my calendar." Every calendar
product worth aggregating already publishes a secret ICS URL for exactly
this read-only use case (Google's "Secret address in iCal format", Apple's
public calendar URLs, most club/school systems, and the Dutch municipal
afvalkalender). A feed URL is a complete MVP boundary: it gives Amity read
access to real calendars with a fraction of the integration cost, and it
structurally cannot write back — there is no API surface to misuse even by
accident. If two-way sync is ever wanted, it is a deliberate, separately
justified expansion of scope, not a natural next step from here.

### First outbound network call — egress guards

`amity-service::feeds::fetch` is the **only** code path in the system that
opens a network socket to anything other than loopback. Because a feed URL
is data the household typed or pasted in — effectively arbitrary, and in the
case of a leaked/shared personal-calendar URL, attacker-influenceable — the
fetch path treats every feed host as untrusted and applies guards at every
layer where an untrusted response could do damage:

- **Scheme allow-list.** `Calendar` construction normalises the URL
  (`amity-core::calendar::normalise_feed_url`) before it is ever stored:
  `webcal://` is rewritten to `https://` (the two schemes are equivalent by
  convention — `webcal` only ever signals "hand this to a calendar client"),
  and any URL that is not `http://` or `https://` — `file://`, `ftp://`, a
  bare host, an empty string — is rejected at build time with
  `CalendarError::InvalidUrl`. The fetch layer never has to defend against a
  non-HTTP scheme because one can never reach storage in the first place.
- **20-second client timeout**, covering connect through reading the whole
  body. A feed host that is slow or has silently dropped the connection must
  not be able to block the 6-hourly sync job indefinitely.
- **5 MiB response cap, enforced while streaming**, not after buffering the
  whole response. `fetch` accumulates chunks and checks the running total
  before extending the buffer, so a feed that is a single enormous chunk
  still aborts promptly instead of allocating the whole thing first. This
  cap is deliberately **not** based on the response's `Content-Length`
  header — a hostile or broken server can omit that header or lie about it,
  and the streaming check catches both.
- **Bounded redirect chain — 5 hops** (`reqwest::redirect::Policy::limited(5)`).
  A normal feed migration (a calendar host moving behind a CDN) is one or
  two hops; five is generous headroom without leaving the door open to an
  unbounded or looping redirect chain.
- **No compression, by construction.** The workspace's `reqwest` dependency
  is built with `default-features = false` and only the `rustls-tls` and
  `json` features enabled — no `gzip`/`brotli`/`deflate` decompression is
  compiled in. This closes the gap the 5 MiB streaming cap would otherwise
  leave open: without this, a malicious feed could serve a tiny compressed
  body that decompresses into gigabytes (a "zip bomb" for HTTP), and the
  cap — which measures bytes as read from the wire — would never see the
  inflated size before it exhausted memory.
- **No household data leaves the device.** A sync is a plain `GET` of the
  feed's stored URL: no query parameters, no request body, no household
  identifiers, no telemetry. The only "outbound" content is the URL itself,
  which the household supplied and which the fetch never modifies (beyond
  the scheme normalisation above).

Together these mean the categorical "no surveillance vectors, no commercial
data flow" commitment (brief §2) is preserved even though the service now
opens outbound sockets. The commitment was never "the process never makes a
network call" — it is "nothing the household didn't ask for leaves the
device, and nothing that leaves is telemetry or profiling." A user-initiated
or user-scheduled read of a URL the household chose satisfies that; a
gzip-bombable, unbounded, unauthenticated fetch of an arbitrary host would
not have, which is why every guard above exists before this ADR is filed
rather than after an incident.

The project-map `privacy` node's "no outbound data flow" claim is corrected
by this task to the precise statement: loopback-only service, plus
user-initiated and scheduled read-only outbound fetches of configured
calendar feeds, bounded by the guards above.

**Accepted out of scope for the MVP:** the guards above bound size, time, and
redirects, but do not add an egress allow-list — within the local-first,
single-household model, a feed URL pointing at loopback/link-local/RFC-1918
internal addresses, and a cleartext-`http` feed exposing its embedded token
on the wire, are both accepted risks, since the household configures its own
feed URLs on its own device rather than a third party supplying them.

### Feed URLs are stored in plaintext

`calendars.url` is a plain `TEXT` column, added in migration `0004`, with no
column-level encryption. This matters because a personal calendar's ICS URL
(Google's "Secret address in iCal format", Apple's public-calendar link)
embeds a long-lived bearer token in the URL itself — anyone who obtains the
URL can read that calendar without further authentication.

**Why plaintext is the right call, not a shortcut:** Amity is local-first
(ADR-0001) — the entire SQLite database lives on the household's own device,
the service binds to loopback only, and nothing about the threat model
distinguishes the `calendars.url` column from every other column in the
database (a task's title, an inbox item's raw text, a member's name). If an
attacker has read access to the SQLite file, they already have the
household's full task list, inbox, and event data — encrypting one column
while a hundred others sit in plaintext protects nothing a real attacker
would be stopped by, while adding real cost: a key that must live somewhere
(and if it lives on the same device, it is not meaningfully separated from
the data it protects), and a decrypt step on every read of a hot path
(the sync job fetches every enabled calendar's URL every cycle). That is
security theatre — the appearance of a control without the substance of one.

If at-rest encryption of the database is wanted later, it belongs at the
database or disk layer (SQLCipher, a LUKS-encrypted disk on the eventual
home-node hardware, OS-level file encryption) where it protects the whole
file uniformly, not as a bespoke per-column scheme invented for this one
task. This is consistent with the local-first architecture: the device
itself is the trust boundary, not any individual table.

### External RRULE expansion uses the full `rrule` crate, not the native validator

`amity-core::recurrence::RecurrenceRule` (ADR-0002) validates a strict RRULE
subset — `DAILY`/`WEEKLY`/`MONTHLY`/`YEARLY` with a limited set of
modifiers — because it exists to keep **native, Amity-authored** recurrence
simple and to reject pathological rules (secondly/minutely) before they can
produce millions of instances. That validator is the wrong tool for external
feeds: a school, club, or Google/Apple calendar is free to emit any RRULE
RFC 5545 allows, and a read-only aggregator that rejected or mis-rendered
real-world feeds because they used `BYSETPOS` or `BYWEEKNO` would defeat the
whole point of aggregating them.

`amity-core::ics::expand_external` therefore expands external RRULEs with
the full `rrule` crate directly — the same crate the native engine is built
on, used here without the subset restriction — so an external feed's
recurrence renders exactly as the source calendar intends. This is safe
specifically because these feeds are **read-only** and bounded by the same
window-based expansion (and the sync job's 6-hourly cadence, response-size
cap, and per-instance materialisation limits) that already bounds native
recurrence; nothing about accepting a wider RRULE grammar reopens the
unbounded-instance concern the native validator exists to prevent, because
external instances are still materialised into a finite window, not
expanded without limit.

---

## Consequences

- Amity can never modify a household member's Google/Apple/school/club
  calendar — by construction, not by policy. If two-way sync is wanted
  later, it requires a new, explicitly-scoped OAuth integration; nothing
  here needs to be un-built to add it.
- The service now makes outbound network calls, which is a durable change
  to the "loopback-only, nothing leaves the device" framing used since
  Task 1. The project map's `privacy` node is updated (Task 6) to state the
  precise boundary — read-only, user-configured, guarded — rather than
  retract the local-first claim.
- `feeds::fetch`'s four guards (scheme allow-list at construction, 20s
  timeout, 5 MiB streamed cap, 5-redirect bound) plus the no-compression
  build must all move together if any of them is ever revisited — they are
  independent layers, not redundant, and removing one (e.g. adding gzip
  support for a "slow feed" complaint) reopens exactly the memory-exhaustion
  risk the streaming cap was built to close.
- A leaked `calendars.url` value (e.g. a backup of the SQLite file handled
  carelessly) exposes the same personal-calendar token exposure a leaked
  browser bookmark or shared link would. This is an accepted, documented
  risk consistent with the local-first threat model, not an oversight.
- External feeds can use any RRULE construct the `rrule` crate supports,
  even ones the native engine deliberately rejects. A future task that adds
  write access to *any* external source (not currently planned) would need
  to reintroduce subset validation for whatever it writes — read-only
  ingestion carries no such obligation.
