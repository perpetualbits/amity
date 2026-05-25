// lib.rs — amity-storage public API.
//
// This crate owns all persistence concerns for Amity:
//   • Database connection pool setup and migration application.
//   • Repository functions per entity (inbox, task, completion_log, task_instance).
//
// The storage layer is deliberately dumb — no business logic lives here.
// Business logic belongs in amity-core; the service layer (amity-service)
// orchestrates calls between the two.
//
// All public functions accept a `&SqlitePool` rather than a wrapper type so
// that callers control connection-pool lifecycle and tests can spin up a
// fresh pool against a temp file without any global state.
//
// Modules:
//   connection      — pool construction and migration application
//   inbox           — repository functions for InboxItem
//   task            — repository functions for Task
//   completion_log  — repository functions for CompletionLog (append-only)
//   task_instance   — repository functions for materialised task instances

/// Database connection pool construction and migration application.
pub mod connection;

/// Repository functions for [`amity_core::inbox::InboxItem`].
pub mod inbox;

/// Repository functions for [`amity_core::task::Task`].
///
/// Exposes insert, fetch, list (with filter), update, and mark-done/skipped.
pub mod task;

/// Repository functions for [`amity_core::completion_log::CompletionLog`].
///
/// Append-only — there is no update or delete path.
pub mod completion_log;

/// Repository functions for materialised `task_instances` rows.
///
/// Exposes bulk upsert, upcoming-instances query, and pruning/deletion helpers
/// used by the recurrence materialisation background job.
pub mod task_instance;

// Re-export the error type at the crate root so callers only need one import.
pub use error::StorageError;

/// Error types for the storage layer.
///
/// Every public function in this crate returns `Result<_, StorageError>`.
/// The storage error wraps sqlx errors but never exposes them raw — that
/// would tie callers to sqlx's internal error structure unnecessarily.
pub mod error {
    /// Errors produced by amity-storage repository functions.
    #[derive(Debug, thiserror::Error)]
    pub enum StorageError {
        /// A sqlx database operation failed.
        ///
        /// The inner error is the raw sqlx error. Callers that want to
        /// distinguish "not found" from "constraint violation" should inspect
        /// `sqlx::Error` variants; most callers treat this as opaque.
        #[error("database error: {0}")]
        Database(#[from] sqlx::Error),

        /// A sqlx migration failed.
        ///
        /// `MigrateError` is a distinct type from `sqlx::Error` even though
        /// it represents a database failure. We wrap it separately so the error
        /// message is clear about the migration context.
        #[error("migration error: {0}")]
        Migration(#[from] sqlx::migrate::MigrateError),

        /// A value stored in the database could not be parsed back into a
        /// domain type. This indicates either a bug in the write path or
        /// a migration that changed a column's set of valid values.
        #[error("parse error reading from database: {0}")]
        Parse(String),
    }
}
