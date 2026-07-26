# Task 5 — ICS ingestion and external calendar aggregation

*Design spec. Status: approved for planning (2026-07-26). Follows Task 4, which
shipped the native `Event` entity, `EventOverride`, storage, the event API, and
surfacing onto Today. Task 4 deliberately deferred external-calendar ingestion
to this task, laying the `EventSource::Ics` variant, the `read_only` flag, and
the `source_*` columns as the seam this task fills.*

## 1. Goal

Make Amity a calendar **aggregator** (brief §7): fetch read-only external ICS
feeds — school calendars per child, sports/club feeds, the municipal
afvalkalender, the public NL holiday calendar, adult members' personal
Google/Apple calendars — parse them, and surface their events on Today alongside
tasks and native events. Amity never writes back to an external source; the only
writable calendar is the hub-native one from Task 4.

The payoff: a household adds a feed URL and its events appear on Today, refreshed
automatically, degrading calmly when a feed is unreachable.

## 2. Scope

### In scope

- A `Calendar` domain entity representing one subscribed feed.
- Pure ICS parsing (text → parsed events) and external-recurrence expansion in
  `amity-core`, unit-testable against fixture `.ics` strings with no network.
- Storage: migration `0004`, a `calendars` repository, and idempotent
  upsert/prune helpers for a feed's events on the existing events tables.
- Outbound HTTP fetch (reqwest) with safety guards, and a background sync job
  mirroring `recurrence_horizon`.
- A calendars HTTP API (add / list / get / enable-disable / delete / refresh).
- ADR-0004 documenting the outbound-egress posture, and a project-map sync.
- Tests at every layer, including a network-free end-to-end surfacing test.

### Out of scope (deliberate)

- **Applying `Reschedule`/`Annotate` overrides** to external instances — this
  remains the Task 4 carry-over. `Cancel` already removes an instance from
  surfacing; the other two are recorded but not yet applied.
- **A feed-management UI** beyond an optional read-only "Calendars" list in the
  hub. Adding/removing feeds through a polished settings view is its own slice
  and cannot be run live here (no WebKit2GTK in this environment).
- **OAuth to Google/Apple.** External calendars are read-only ICS URLs only,
  ever (brief §7). A personal calendar is subscribed via its secret ICS URL.
- **Write-back to any external source.** The entire point of `EventOverride`.
- **Per-instance `RECURRENCE-ID` overrides, `VALARM`, `VTIMEZONE` construction,
  and `RDATE`.** See §5 for the parsing subset.

## 3. Architecture

The work follows the codebase's existing three-layer separation. Nothing about
ingestion is allowed to blur it.

```
amity-core     (pure, no I/O)      Calendar type · ics::parse_feed · expand_external
amity-storage  (sqlx → SQLite)     migration 0004 · calendars repo · event upsert/prune
amity-service  (axum + jobs)       feeds::fetch · jobs::calendar_sync · api/calendar
```

Data flow for one sync of one calendar:

```
fetch(url) ──▶ ics::parse_feed(text) ──▶ map to Events ──▶ upsert_external_events
   │                                                            │
   │                                              prune_events_missing_from_feed
   ▼                                                            │
record sync status                              expand_external ─▶ materialise instances
```

Surfacing is unchanged: once external events and their instances are in the
events tables, Task 4's `build_event_candidates` already draws them into the
ranked Today query. External events surface on their day and are never overdue,
exactly like native events.

**Rejected alternatives.** A dedicated `amity-calendar` crate — unnecessary
scaffolding for the current size; it would also pull reqwest/tokio into a new
crate. A service-only implementation with no core involvement — parsing logic
becomes hard to unit-test and breaks the pure-core discipline the codebase
values.

## 4. The `Calendar` entity (amity-core)

```
Calendar {
    id: CalendarId,           // new UUID-v7 newtype in ids.rs
    name: String,             // display name, non-empty (trimmed)
    url: String,              // http(s) feed URL; webcal:// normalised to https://
    category: CalendarCategory,
    enabled: bool,            // disabled feeds are skipped by the sync job
    created_at: OffsetDateTime,
}

CalendarCategory = School | Club | Waste | Holiday | Personal | Other
```

Construction goes through a builder with an injected `now` clock, matching every
other entity in the codebase. Validation: name non-empty after trimming; URL
parses and its scheme is `http`, `https`, or `webcal` (the last rewritten to
`https`); category defaults to `Other`. `CalendarCategory` gets `Display`/
`FromStr` for snake_case storage strings, like the other enums.

The **sync state** — `last_synced_at`, `last_status`, `last_error`,
`event_count` — is *not* part of the immutable `Calendar` value. It lives on the
storage row and is updated by the job; the repository returns it as a separate
`CalendarSyncState` struct (or a combined read model) so the domain type stays a
clean description of the subscription, not its runtime status.

## 5. ICS parsing and expansion (amity-core, pure)

Two pure, I/O-free functions, both fully unit-testable with fixture strings:

**`parse_feed(text: &str) -> Result<Vec<ParsedEvent>, IcsError>`**

```
ParsedEvent {
    uid: String,              // VEVENT UID → becomes source_external_id
    summary: String,          // SUMMARY → title (empty/absent → "(untitled event)")
    start: OffsetDateTime,
    end: Option<OffsetDateTime>,
    all_day: bool,            // DTSTART;VALUE=DATE → true
    rrule: Option<String>,    // raw RRULE line, expanded later
    exdates: Vec<OffsetDateTime>,
    tzid: Option<String>,     // TZID of DTSTART, carried for display
}
```

- The concrete ICS parser crate (`icalendar` or `ical`, settled at
  implementation) is an **internal detail** hidden behind `parse_feed`, so the
  choice is swappable and the rest of the system depends only on `ParsedEvent`.
- **Resilience:** a malformed `VEVENT` is logged and skipped; the rest of the
  feed still parses. A feed with zero usable events is a valid (empty) result,
  not an error. `IcsError` is reserved for a feed that is not iCalendar at all.
- **Subset:** `SUMMARY`, `DTSTART`/`DTEND`, all-day (`VALUE=DATE`), `TZID`,
  `RRULE`, `EXDATE`, `UID`. Ignored: `VALARM`, `RECURRENCE-ID`, `RDATE`,
  attendee/organizer fields, `VTODO`/`VJOURNAL` components.

**`expand_external(parsed: &ParsedEvent, from, to) -> Vec<OffsetDateTime>`**

- Takes an explicit `[from, to]` instant window (the caller passes
  `[now, now + 60 days]` — the same 60-day forward horizon tasks and native
  events use). The window is a parameter, not a hard-coded constant, so it stays
  pure and testable.
- Uses the **full `rrule` crate directly**, not the native-recurrence validator
  in `recurrence.rs`. That validator intentionally rejects `BYSETPOS`,
  `BYWEEKNO`, sub-daily `FREQ`, etc. to keep *native* recurrence simple — but
  external feeds are read-only and must render whatever real calendars emit, so
  expansion bypasses the subset gate.
- Applies `EXDATE` by removing matching instants.
- A non-recurring event expands to its single `start` instant when that instant
  falls within the window, otherwise to nothing.

## 6. Storage (amity-storage)

**Migration `0004_add_calendars.sql`** — a `calendars` STRICT table:

| column | type | notes |
|---|---|---|
| id | TEXT PK | UUID v7 |
| name | TEXT | non-empty |
| url | TEXT | http(s) feed URL |
| category | TEXT | snake_case enum |
| enabled | INTEGER | 0/1 |
| created_at | TEXT | RFC 3339 |
| last_synced_at | TEXT NULL | set by the job |
| last_status | TEXT | `never` \| `ok` \| `unreachable` \| `parse_error` |
| last_error | TEXT NULL | short diagnostic on failure |
| event_count | INTEGER | events from the last good sync |

**`calendars` repository:** `insert`, `list`, `fetch`, `update` (enable/disable),
`delete` (cascades to the feed's events and instances), and
`update_sync_state(id, status, error, count, synced_at)`.

**External-event upsert/prune** on the existing events tables — a feed owns its
event set, so re-sync is a full reconciliation:

- `upsert_external_events(calendar_id, events)` — insert-or-**update** keyed on
  `(source_calendar_id, source_external_id)`. Unlike the idempotent
  `INSERT OR IGNORE` used elsewhere, this must UPDATE changed fields (a feed
  event can be edited upstream). Migration `0004` adds the unique index on
  `(source_calendar_id, source_external_id)` over the events table from `0003`
  that makes this `ON CONFLICT … DO UPDATE` well-defined.
- `prune_events_missing_from_feed(calendar_id, keep_external_ids)` — delete this
  calendar's events (and their instances) whose UID is no longer in the feed.
- Instances are re-materialised from `expand_external` after upsert, reusing the
  `event_instances` upsert path from Task 4.

## 7. Fetch and sync job (amity-service)

**`feeds::fetch(url) -> Result<String, FetchError>`** via reqwest, with guards
appropriate to the first outbound call in the system:

- `http`/`https` only (scheme already normalised at entity construction).
- A request **timeout** (default 20s) and a **response-size cap** (default
  5 MiB, streamed and aborted past the cap) so a hostile or broken feed cannot
  hang or exhaust memory.
- A **bounded redirect** count.
- No household data in the request — a plain `GET` of a user-configured URL.

**`jobs::calendar_sync`** mirrors `jobs::recurrence_horizon`:

- `run_once(pool, fetch_fn)` iterates **enabled** calendars. For each: fetch →
  `parse_feed` → map to `Event`s (source = `EventSource::ics(uid, calendar_id,
  now)`) → `upsert_external_events` → `prune_events_missing_from_feed` →
  materialise instances → `update_sync_state`.
- `fetch_fn` is **injected**, so tests pass a closure returning fixture ICS text
  and the job runs with zero network.
- `spawn(pool)` runs one pass on startup and then every ~6h on a tokio interval.
- **Resilience:** each calendar is isolated — a fetch failure records
  `unreachable` + the error and **keeps the last-good events** (never wipes on
  failure); a parse failure records `parse_error`; one bad feed never affects
  another. Individual malformed VEVENTs are already dropped inside `parse_feed`.

## 8. Calendars API (amity-service)

All under `/api/v1/`, following the existing handler conventions (422 on
validation failure, 400 on a malformed id, 404 on unknown id):

| method + path | purpose |
|---|---|
| `POST /calendars` | subscribe a feed `{ name, url, category? }` → 201 with the row |
| `GET /calendars` | list all feeds with sync state |
| `GET /calendars/{id}` | one feed with sync state |
| `PATCH /calendars/{id}` | toggle `enabled` |
| `DELETE /calendars/{id}` | unsubscribe — removes the feed and its events/instances |
| `POST /calendars/{id}/refresh` | run a sync for this feed now (on-demand) |

`POST` does **not** block on a first fetch; it creates the feed with
`last_status = never` and lets the job (or an explicit refresh) populate it, so
adding a slow or unreachable feed never hangs the request.

## 9. Surfacing

No code change. External events and their instances land in the same events
tables Task 4's `build_event_candidates` already reads, so they surface on Today
through the one kind-agnostic ranking rule. External events surface on their
start day and are never overdue, identical to native events. The Today view's
existing event kind-marker (◆) already covers them.

*(A future enhancement — showing the source calendar or category on a surfaced
item — is explicitly not part of this task.)*

## 10. Privacy and egress (ADR-0004)

This task introduces the **first outbound network call** in Amity. Until now the
service is loopback-only and no data leaves the device — a categorical
commitment (brief §2) and a claim on the project map. Fetching an ICS feed is a
legitimate, user-initiated **read**: no household data is transmitted; the
request is a plain `GET` of a URL the household chose. The commitment being
honoured is "no *surveillance* vectors and no *commercial* data flow," not
"never open a socket."

ADR-0004 records:

- The aggregator posture and why read-only ICS (not OAuth) is the MVP boundary.
- The egress guards from §7 (scheme allow-list, timeout, size cap, redirect
  bound), and that requests carry no household data.
- That feed URLs — which for personal calendars embed **secret tokens** — are
  stored in the local SQLite DB in **plaintext**, consistent with local-first
  (the entire DB is on-device, the service is loopback-only). Encrypting a
  single column while the rest of the DB is plaintext would be security theatre;
  if at-rest encryption is wanted later it belongs at the database/disk layer,
  not one column.

The project-map `privacy` node is updated from the blanket "no outbound data
flow" to the precise, honest statement: loopback-only service, plus user-
initiated read-only outbound fetches of configured calendar feeds.

## 11. Errors

- `IcsError` (core) — the feed is not iCalendar. Individual bad VEVENTs do not
  raise it; they are skipped.
- `FetchError` (service) — network failure, timeout, oversize, non-2xx status.
- `SyncStatus = Never | Ok | Unreachable | ParseError` — persisted per calendar,
  surfaced through the API so the (future) UI and the operator can see why a
  feed is stale without reading logs.
- The builder and repository reuse the existing `thiserror` enum patterns.

## 12. Testing

- **core (pure, fixtures):** `parse_feed` on a single timed event, an all-day
  event, a recurring event with `RRULE`, one with `EXDATE`, a feed with a
  malformed VEVENT among good ones (skipped), a non-calendar payload
  (`IcsError`), and a `TZID`-bearing event. `expand_external` over the horizon,
  including EXDATE removal and a full-RRULE feature our native validator would
  reject (proving the bypass).
- **storage (integration):** calendars CRUD; `upsert_external_events` inserting
  then updating a changed event; `prune_events_missing_from_feed`; the unique
  `(calendar_id, external_id)` index.
- **service (integration, network-free):** the calendars API surface; and the
  **end-to-end** path — add a calendar, run `calendar_sync::run_once` with an
  injected fixture feed, assert the event surfaces on `/surfacing/today`, then
  re-sync with that event removed from the feed and assert it disappears; plus a
  fetch-failure run that records `unreachable` and keeps prior events.

Every production `crates/**` file continues to meet the comment-density gate
(now string-literal-aware, so embedded ICS fixtures and SQL do not inflate the
code count).

## 13. Rough slicing

Detailed sequencing comes from the writing-plans step; the intended shape:

1. `Calendar` type + `CalendarId` + ICS `parse_feed`/`expand_external` (core, TDD).
2. Migration `0004` + calendars repository + external-event upsert/prune (storage).
3. `feeds::fetch` + `jobs::calendar_sync` with injectable fetch (service).
4. Calendars API + the network-free end-to-end surfacing test (service).
5. ADR-0004 + project-map sync (+ optional read-only Calendars list in the hub).

## 14. Estimate

6–9 focused days for slices 1–5 without the optional hub list, consistent with
the Task 4 doc's projection for the deferred ingestion sub-project.
