// ids.rs — typed ID newtypes for all Amity domain entities.
//
// Every entity uses a distinct newtype over `Uuid` rather than a bare `Uuid`.
// This makes it a compile-time error to pass an `InboxItemId` where a
// `MemberId` is expected — a whole class of logic bugs eliminated for free.
//
// IDs use UUID v7 (time-ordered) rather than v4 (random). Time-ordering
// keeps related rows physically close in the B-tree index, which matters
// at the scale of years of household data. See ADR-0001.
//
// All ID types derive `serde::{Serialize, Deserialize}` so they round-trip
// cleanly through the JSON API and the SQLx storage layer.

// Serde derives are used on every ID newtype for JSON serialisation.
use serde::{Deserialize, Serialize};
// Uuid is the inner type for all IDs; imported here so the macro body can use it.
use uuid::Uuid;

/// Generate a fresh time-ordered UUID v7.
///
/// Centralising generation here means tests can verify that IDs are non-empty
/// without caring about the specific value, and future refactors to a
/// monotonic clock source need only change this one function.
fn new_id() -> Uuid {
    // v7 uses the current system time as the high bits, then random bits for
    // uniqueness within the same millisecond. Globally unique and sortable.
    Uuid::now_v7()
}

// ─── Macro for boilerplate-free newtype IDs ──────────────────────────────────
//
// Each newtype needs the same set of derives and the same `new()` constructor.
// The macro eliminates the repetition while keeping the types completely distinct
// at the type-system level — `impl From<InboxItemId> for MemberId` does not exist.

macro_rules! define_id {
    (
        // Accept zero or more doc-comment / attribute tokens so callers can
        // document each ID type with `///` lines before the type name.
        $(#[$meta:meta])*
        $name:ident
    ) => {
        // Apply the caller-provided attributes (typically `///` doc comments).
        // The `*` allows zero attributes so a bare `define_id!(Foo)` also works.
        $(#[$meta])*
        // Debug: useful in test failure messages.
        // Clone/Copy: IDs are small enough to copy cheaply.
        // PartialEq/Eq/Hash: needed for use as HashMap keys and in assertions.
        // Serialize/Deserialize: required for JSON API and sqlx TEXT round-trips.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        // Transparent representation so sqlx can bind/read the inner Uuid directly
        // without a wrapping struct in the JSON/SQL output.
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a fresh, globally unique ID.
            #[must_use]
            pub fn new() -> Self {
                // Delegate to `new_id()` so the UUID version is controlled in
                // one place; swapping v7 for a monotonic source requires only
                // changing that function.
                Self(new_id())
            }
        }

        impl Default for $name {
            /// Generates a new random ID, consistent with `new()`.
            ///
            /// `Default` is implemented so that builder patterns and test
            /// helpers can call `..Default::default()` without surprising
            /// anyone about what "default" means here.
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            /// Format the ID as a hyphenated UUID string.
            ///
            /// This is the canonical form used in JSON, URLs, and database TEXT
            /// columns — e.g. `"018f1a2b-dead-beef-cafe-000000000001"`.
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                // Hyphenated form is the canonical UUID string representation
                // used in JSON, URLs, and database TEXT columns.
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            /// Parse a UUID string into this ID type.
            ///
            /// # Errors
            ///
            /// Returns `uuid::Error` if the string is not a valid UUID.
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                // Accept any standard UUID string format (hyphenated, simple, braced).
                Ok(Self(s.parse()?))
            }
        }
    };
}

define_id!(
    /// Unique identifier for an [`InboxItem`](crate::inbox::InboxItem).
    ///
    /// See brief §6.3 for the Inbox data model.
    InboxItemId
);

define_id!(
    /// Unique identifier for a household member.
    ///
    /// The Member entity itself is not yet implemented (it arrives in a later
    /// task). The ID type exists now so that `InboxItem.captured_by` has a
    /// proper typed reference rather than a bare `Uuid`. See brief §6.5 for
    /// the full Member model.
    MemberId
);

define_id!(
    /// Unique identifier for a [`Task`](crate::task::Task).
    ///
    /// See brief §6.5 for the Task data model. Time-ordered UUID v7 keeps
    /// related task rows physically adjacent in the B-tree index.
    TaskId
);

define_id!(
    /// Unique identifier for a [`CompletionLog`](crate::completion_log::CompletionLog).
    ///
    /// Each completed or skipped task instance writes one `CompletionLog` row.
    /// See brief §6.5 (`CompletionLog`) and §8.2 (chore-completion model).
    CompletionLogId
);

define_id!(
    /// Stub identifier for a Project entity.
    ///
    /// The Project entity is not yet implemented (Task 2 scope guardrails).
    /// The ID type is declared now so that `Task.project_id` has a proper
    /// typed `Option<ProjectId>` rather than a bare `Option<Uuid>`. The
    /// field is always `None` until the Project entity lands.
    ProjectId
);

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_item_id_new_is_unique() {
        // Each call to `new()` must produce a distinct ID.
        // If this ever fails, the UUID source is broken.
        let a = InboxItemId::new();
        let b = InboxItemId::new();
        assert_ne!(a, b, "consecutive IDs must be distinct");
    }

    #[test]
    fn member_id_new_is_unique() {
        // Same guarantee as `inbox_item_id_new_is_unique` — each type is tested
        // separately because the macro generates distinct implementations.
        // Two calls in the same millisecond must still produce distinct IDs
        // (UUID v7 adds random bits after the timestamp for this reason).
        let a = MemberId::new();
        let b = MemberId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn id_round_trips_through_string() {
        // IDs are stored as TEXT in SQLite; the FromStr/Display round-trip
        // must be lossless so reads equal the original written value.
        let id = InboxItemId::new();
        let s = id.to_string();
        let parsed: InboxItemId = s.parse().expect("valid UUID string");
        assert_eq!(id, parsed);
    }

    #[test]
    fn id_round_trips_through_json() {
        // Transparent serde means the JSON value is just the UUID string,
        // not an object. Verify the wire format is what consumers expect.
        let id = MemberId::new();
        let json = serde_json::to_string(&id).expect("serialise");
        // The JSON should be a quoted string, not {"0": "..."}.
        assert!(json.starts_with('"'), "expected a JSON string, got: {json}");
        let back: MemberId = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(id, back);
    }

    #[test]
    fn inbox_item_id_and_member_id_are_distinct_types() {
        // Compile-time check: the macro generates separate types.
        // This test is a documentation assertion — if it compiles, the
        // types are distinct. The body just validates the types are usable.
        let _inbox: InboxItemId = InboxItemId::new();
        let _member: MemberId = MemberId::new();
        // The following would be a compile error (intentionally omitted):
        // let _: InboxItemId = _member;
    }

    #[test]
    fn task_id_new_is_unique() {
        // Each call to `new()` must produce a distinct ID — same guarantee as
        // the inbox/member tests, verified separately per the macro's contract.
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b, "consecutive TaskIds must be distinct");
    }

    #[test]
    fn completion_log_id_new_is_unique() {
        // CompletionLogId is a distinct newtype; verify its uniqueness guarantee.
        // Tested separately from `InboxItemId` and `MemberId` because the macro
        // generates independent implementations that must each be correct.
        let a = CompletionLogId::new();
        let b = CompletionLogId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn project_id_stub_is_usable() {
        // ProjectId is a stub — no Project entity yet, but the type must be
        // constructable so Task.project_id = None compiles without an explicit
        // type annotation at the call site.
        let _: Option<ProjectId> = None;
        // Creating one must not panic — the UUID source is always available.
        let _id = ProjectId::new();
    }
}
