// jobs/mod.rs — background maintenance jobs.
//
// Jobs are long-running or periodic tasks that keep derived data fresh without
// a client request driving them. Each job lives in its own sub-module and is
// spawned from `main` after the database is opened.
//
// Modules:
//   recurrence_horizon — extend materialised task instances and prune old ones.

/// The recurrence horizon maintenance job (see ADR-0002 §materialisation-strategy).
pub mod recurrence_horizon;
