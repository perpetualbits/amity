// jobs/mod.rs — background maintenance jobs.
//
// Jobs are long-running or periodic tasks that keep derived data fresh without
// a client request driving them. Each job lives in its own sub-module and is
// spawned from `main` after the database is opened.
//
// Modules:
//   recurrence_horizon — extend materialised task instances and prune old ones.
//   calendar_sync      — fetch subscribed ICS feeds and ingest their events.

/// The recurrence horizon maintenance job (see ADR-0002 §materialisation-strategy).
pub mod recurrence_horizon;

/// The ICS calendar sync job (see brief §7 and ADR-0004 for the egress guards).
pub mod calendar_sync;
