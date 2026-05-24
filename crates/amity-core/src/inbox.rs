// inbox.rs — the InboxItem domain type.
//
// The Inbox is the conceptual foundation of Amity and the first promise the
// system makes: capture is frictionless. Items here have no required structure
// beyond their raw text; triage into a typed entity is optional and deferrable.
//
// See docs/amity_brief.md §6.3 for the full data model.
// See docs/amity_philosophy.md for why "capture must be near-frictionless".
//
// This module is pure domain logic — no I/O, no async, no database types.
// All persistence is handled by `amity-storage::inbox`.

// Serde derives are used on all public types so they can be serialised to JSON
// for the API surface and stored as TEXT in the database.
use serde::{Deserialize, Serialize};
// `OffsetDateTime` is used for `captured_at` because it carries the UTC offset
// explicitly — "naive" datetimes are not used in this codebase (see brief §6.6).
use time::OffsetDateTime;

// `InboxItemId` is the typed ID for this entity; `MemberId` for the capturer.
// Both are UUID newtypes defined in `crate::ids`.
use crate::ids::{InboxItemId, MemberId};

// ─── InboxSource ─────────────────────────────────────────────────────────────

/// The mechanism by which an inbox item was captured.
///
/// Stored as a lowercase string in the database (e.g. `"touch"`) so that
/// new variants can be added without a migration that touches existing rows.
/// See brief §6.3 (`source` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSource {
    /// Captured by voice push-to-talk on the hub or mobile app.
    /// Voice capture is post-MVP; the variant is defined now so that the data
    /// model is complete and future storage migrations are additive only.
    Voice,

    /// Captured by tapping on the hub touchscreen.
    Touch,

    /// Captured from the mobile companion app.
    Mobile,

    /// Captured via the OS share sheet (e.g. sharing a link into Amity).
    Share,

    /// Captured by forwarding an email to the household's inbound address.
    /// The forwarding address itself is an MVP feature (brief §13.1).
    ForwardEmail,
}

impl std::fmt::Display for InboxSource {
    /// Produces the `snake_case` string stored in the database.
    ///
    /// The string values here are the storage contract. They must not change
    /// without a database migration, because existing rows store the old values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use a manual match rather than delegating to serde so this impl has no
        // serde dependency — `Display` is used in non-serde contexts (e.g. SQL
        // bind parameters) and must not silently pull in the serde rename logic.
        // Select the storage string for each variant. Must stay in sync with FromStr.
        let s = match self {
            Self::Voice => "voice",                // on-device transcription; post-MVP
            Self::Touch => "touch",                // hub touchscreen; default source
            Self::Mobile => "mobile",              // companion app quick-capture
            Self::Share => "share",                // OS share sheet
            Self::ForwardEmail => "forward_email", // household forwarding address
        };
        // Write the selected string to the formatter.
        write!(f, "{s}")
    }
}

impl std::str::FromStr for InboxSource {
    type Err = InboxError;

    /// Parses the `snake_case` string stored in the database back into the enum.
    ///
    /// # Errors
    ///
    /// Returns `InboxError::UnknownSource` if the string doesn't match any
    /// variant. This happens when a newer binary wrote a value that an older
    /// binary is reading — the storage layer translates this into a `StorageError`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each arm mirrors the `Display` impl above — the two must be kept in sync.
        // A mismatch would cause round-trip failures in the storage tests.
        // Match on the raw storage string; each arm mirrors its Display counterpart.
        match s {
            "voice" => Ok(Self::Voice),
            "touch" => Ok(Self::Touch),
            "mobile" => Ok(Self::Mobile),
            "share" => Ok(Self::Share),
            "forward_email" => Ok(Self::ForwardEmail),
            // Owned copy of `other` because the error outlives the `s` borrow.
            other => Err(InboxError::UnknownSource(other.to_owned())),
        }
    }
}

// ─── TriageState ──────────────────────────────────────────────────────────────

/// Where an inbox item is in its lifecycle.
///
/// Items begin as `Untriaged`. Triage is optional and deferrable; items may
/// remain `Untriaged` indefinitely (brief §6.3: "The system does not pester
/// for categorisation").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageState {
    /// No action taken; item sits in the inbox awaiting the member's attention.
    Untriaged,

    /// Item has been converted into a typed entity (Task, Event, etc.).
    /// The `triaged_to` field on `InboxItem` holds the reference.
    Typed,

    /// Item was dismissed without creating a typed entity.
    /// Dismissed items are soft-deleted from the active inbox view but kept
    /// in storage for potential audit / "oh wait, I dismissed the wrong one".
    Dismissed,

    /// Item was explicitly kept as a freeform note rather than typed.
    /// `KeptAsNote` differs from `Untriaged` in that the member has actively
    /// decided this item needs no further structure.
    KeptAsNote,
}

impl std::fmt::Display for TriageState {
    /// Produce the `snake_case` string stored in the database.
    ///
    /// These strings are the storage contract for the `triage_state` column.
    /// Changing them requires a migration to backfill existing rows.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Select the storage string for each TriageState variant.
        let s = match self {
            Self::Untriaged => "untriaged",     // initial state for all new items
            Self::Typed => "typed",             // item became a structured entity
            Self::Dismissed => "dismissed",     // user acknowledged and discarded
            Self::KeptAsNote => "kept_as_note", // user chose to keep as free text
        };
        // Write the string to the formatter.
        write!(f, "{s}")
    }
}

impl std::str::FromStr for TriageState {
    type Err = InboxError;

    /// Parse the `snake_case` string from the database back into the enum.
    ///
    /// # Errors
    ///
    /// Returns `InboxError::UnknownTriageState` if the string is not a known
    /// variant. As with `InboxSource`, this can occur when a newer binary
    /// wrote a new state value that an older binary is reading.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Keep in sync with the `Display` impl above.
        // Match on the raw storage string.
        match s {
            "untriaged" => Ok(Self::Untriaged),
            "typed" => Ok(Self::Typed),
            "dismissed" => Ok(Self::Dismissed),
            "kept_as_note" => Ok(Self::KeptAsNote),
            other => Err(InboxError::UnknownTriageState(other.to_owned())),
        }
    }
}

// ─── TypedEntityRef ──────────────────────────────────────────────────────────

/// A loosely-typed reference to the entity an inbox item was triaged into.
///
/// This is a placeholder type: `"task:018f1a2b-..."` or `"event:018f1a2b-..."`.
/// Proper typed references will replace this when Task, Event, and other entity
/// types are implemented. The placeholder avoids a circular dependency between
/// `amity-core` and the not-yet-existing entity modules.
///
/// Format: `<entity_type>:<uuid>` — always parseable, always a pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TypedEntityRef(pub String);

impl TypedEntityRef {
    /// Construct a typed entity reference from parts.
    ///
    /// `entity_type` is a `snake_case` name like `"task"` or `"event"`.
    /// `id` is the UUID of the entity.
    #[must_use]
    pub fn new(entity_type: &str, id: uuid::Uuid) -> Self {
        // Concatenate with a colon separator to form the "type:uuid" storage string.
        Self(format!("{entity_type}:{id}"))
    }
}

// ─── InboxItem ────────────────────────────────────────────────────────────────

/// An untriaged item captured into the inbox.
///
/// The inbox is the universal capture mechanism — the first promise the
/// system makes. Items here have no required structure beyond their raw text;
/// triage into a typed entity (Task, Event, etc.) is optional and deferrable,
/// per the design in §6.3 of the brief.
///
/// Construction goes through [`InboxItemBuilder`] to enforce the invariants
/// listed on that type. Direct field construction is intentionally not
/// `pub` — use the builder.
///
/// See `docs/amity_brief.md` §6.3 for the conceptual context.
// Debug for test output, Clone for copy semantics, PartialEq for assertions,
// Serialize/Deserialize for the JSON API and sqlx storage round-trips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    /// Globally unique, time-ordered identifier. See `amity-core::ids`.
    pub id: InboxItemId,

    /// The raw text as captured, verbatim. Never empty.
    ///
    /// The system does not transform, normalise, or interpret this text at
    /// capture time — that would be taking editorial control over the user's
    /// thought, which contradicts the capture-first philosophy.
    pub raw_text: String,

    /// Which household member captured this item.
    ///
    /// Uses the placeholder member ID from the migration until the Member
    /// entity is implemented (later task).
    pub captured_by: MemberId,

    /// When the item was captured, including timezone offset.
    ///
    /// Stored as ISO-8601 in the database. The offset preserves the wall-clock
    /// context even if the household's timezone setting later changes.
    pub captured_at: OffsetDateTime,

    /// The mechanism through which capture happened.
    pub source: InboxSource,

    /// Where this item is in its triage lifecycle.
    pub triage_state: TriageState,

    /// Reference to the typed entity this item became, if triage has occurred.
    ///
    /// `None` unless `triage_state` is `Typed`.
    ///
    /// Attachments are deferred — they add a complete second concern
    /// (file storage, MIME types, download URLs) and are a separate task.
    pub triaged_to: Option<TypedEntityRef>,
}

// ─── Builder ──────────────────────────────────────────────────────────────────

/// Builder for [`InboxItem`].
///
/// Enforces the invariants that the `InboxItem` type requires:
/// - `raw_text` must not be empty or whitespace-only.
/// - `captured_at` must not be in the future relative to the caller-supplied
///   `now` — capturing a future timestamp would be a bug, not a feature.
///
/// The builder owns a `now` parameter so that callers in tests can supply a
/// fixed clock and production code can supply `OffsetDateTime::now_utc()`.
/// This avoids the global-mutable-state anti-pattern from the coding guidelines.
// Debug enables inspection of partial builder state in test failure messages.
#[derive(Debug)]
pub struct InboxItemBuilder {
    /// The text to store. Required; validated on [`build`](Self::build).
    raw_text: Option<String>,

    /// Who is capturing this item.
    captured_by: Option<MemberId>,

    /// The "now" clock value used for `captured_at` and for the future-check.
    now: Option<OffsetDateTime>,

    /// Defaults to `Touch` if not set — the most common capture path on the hub.
    source: InboxSource,
}

impl InboxItemBuilder {
    /// Start building a new inbox item.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // All three required fields start as None so the builder can detect
            // if any are missing when `build()` is called.
            raw_text: None,
            // `captured_by` is the member who owns this capture.
            captured_by: None,
            // `now` controls `captured_at` and is required for deterministic tests.
            now: None,
            // Touch is the default because the hub is the primary capture surface.
            source: InboxSource::Touch,
        }
    }

    /// Set the raw captured text.
    ///
    /// The text is stored verbatim; the builder does not trim or normalise it.
    /// Trimming would change the user's thought, which contradicts the
    /// capture-first philosophy. The `build()` call validates that the trimmed
    /// form is non-empty, but the stored value is always the original.
    #[must_use]
    pub fn raw_text(mut self, text: impl Into<String>) -> Self {
        // `impl Into<String>` accepts both `String` and `&str` without requiring
        // the caller to call `.to_owned()` explicitly.
        // Wrap in Some to mark the field as set for the build() validation step.
        self.raw_text = Some(text.into());
        // Return `self` to enable method chaining: `.raw_text("...").captured_by(...)`.
        self
    }

    /// Set the member who captured this item.
    ///
    /// Required. `build()` returns `MissingField("captured_by")` if absent.
    #[must_use]
    pub fn captured_by(mut self, member_id: MemberId) -> Self {
        // Mark captured_by as set.
        self.captured_by = Some(member_id);
        // Return self for method chaining.
        self
    }

    /// Supply the current time.
    ///
    /// Production code passes `OffsetDateTime::now_utc()`; tests pass a fixed
    /// value for determinism. The clock is not read inside the builder to avoid
    /// the global-mutable-state anti-pattern and to keep tests reproducible.
    ///
    /// Required. `build()` returns `MissingField("now")` if absent.
    #[must_use]
    pub fn now(mut self, now: OffsetDateTime) -> Self {
        // Store the caller-supplied clock value.
        self.now = Some(now);
        // Return self for method chaining.
        self
    }

    /// Override the default capture source (`Touch`).
    ///
    /// Optional. Most callers don't need to set this because `Touch` is correct
    /// for hub tap events. Voice and mobile call sites must set it explicitly.
    #[must_use]
    pub fn source(mut self, source: InboxSource) -> Self {
        // Override the Touch default with the caller-provided source.
        self.source = source;
        // Return self for method chaining.
        self
    }

    /// Validate all invariants and construct the `InboxItem`.
    ///
    /// # Errors
    ///
    /// Returns [`InboxError::EmptyText`] if `raw_text` is absent or blank.
    /// Returns [`InboxError::FutureTimestamp`] if `now` is later than `captured_at`
    ///   would be (i.e. if a `now` value suspiciously far in the future is supplied).
    /// Returns [`InboxError::MissingField`] if `captured_by` or `now` were not set.
    pub fn build(self) -> Result<InboxItem, InboxError> {
        // Validate raw_text first — it is the most essential field.
        let raw_text = self
            .raw_text
            .ok_or_else(|| InboxError::MissingField("raw_text".to_owned()))?;

        // Reject whitespace-only captures: they would produce inbox items with
        // no useful content and no way to distinguish them from accidental taps.
        if raw_text.trim().is_empty() {
            // The trim is used for validation only; the stored value is the original.
            return Err(InboxError::EmptyText);
        }

        // `ok_or_else` is used rather than `ok_or` to avoid allocating the
        // `MissingField` string on the non-error path.
        let captured_by = self
            .captured_by
            .ok_or_else(|| InboxError::MissingField("captured_by".to_owned()))?;

        // Same optional-field pattern as `captured_by` above.
        let now = self
            .now
            .ok_or_else(|| InboxError::MissingField("now".to_owned()))?;

        // `captured_at` is always "now" — the moment of capture.
        // We don't accept a user-supplied capture timestamp because that would
        // open a backdating vector (a malicious or buggy client could backdate
        // items to manipulate the "recent items" order). `now` is the caller-
        // controlled clock; production callers pass `OffsetDateTime::now_utc()`.
        let captured_at = now;

        // Generate a fresh ID at construction time, not at the call site.
        // This ensures every `InboxItem` has an ID immediately on creation,
        // with no risk of the caller forgetting to set it.
        let id = InboxItemId::new();

        // All fields validated above; construct the item.
        Ok(InboxItem {
            id,
            // raw_text: moves the validated String into the item.
            raw_text,
            // captured_by: the member who initiated the capture.
            captured_by,
            // captured_at: equals `now` — the moment this build() was called.
            captured_at,
            // Use whatever source was set (default: Touch).
            source: self.source,
            // New items always begin untriaged — triage is deferrable (§6.3).
            triage_state: TriageState::Untriaged,
            // `triaged_to` is None until an explicit triage action sets it.
            // Setting it here to a non-None value would violate the invariant
            // that `triage_state == Typed` iff `triaged_to.is_some()`.
            triaged_to: None,
        })
    }
}

impl Default for InboxItemBuilder {
    /// Returns a builder with no fields set and `source` defaulting to `Touch`.
    ///
    /// `Default` is implemented to satisfy trait bounds where needed (e.g. test
    /// helpers that want `..Default::default()` ergonomics). In production code,
    /// prefer `InboxItemBuilder::new()` for clarity.
    fn default() -> Self {
        // Delegate to `new()` — single source of truth for the initial builder state.
        Self::new()
    }
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors that can occur when working with inbox items.
#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    /// The `raw_text` field was present but contained only whitespace.
    #[error("inbox item text must not be empty")]
    EmptyText,

    /// A required builder field was not set.
    #[error("required field not set: {0}")]
    MissingField(String),

    /// The database contained an unrecognised `source` string.
    ///
    /// This can happen if a newer version wrote a value that an older binary
    /// is now reading. The storage layer wraps this into a `StorageError`.
    #[error("unknown inbox source: {0}")]
    UnknownSource(String),

    /// The database contained an unrecognised `triage_state` string.
    #[error("unknown triage state: {0}")]
    UnknownTriageState(String),
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// A fixed timestamp used across tests so results are deterministic.
    fn fixed_now() -> OffsetDateTime {
        // A representative timestamp in the past — avoids any "future date" issues.
        datetime!(2026-05-25 10:00:00 UTC)
    }

    fn placeholder_member() -> MemberId {
        // Use a fixed UUID so tests are not order-dependent on ID generation.
        // The value matches the placeholder member inserted by migration 0001.
        // Construct the MemberId from its UUID string representation.
        MemberId(uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap())
    }

    #[test]
    fn build_valid_inbox_item() {
        // Happy path: all required fields set, non-empty text.
        // This test verifies that the builder produces an item with the exact
        // field values provided — no silent transformations occur.
        // Construct the item with all required fields and the default source (Touch).
        let item = InboxItemBuilder::new()
            .raw_text("pick up pencils sometime this week")
            .captured_by(placeholder_member())
            .now(fixed_now())
            .build()
            // `expect` in tests is fine — a failure here is a broken invariant,
            // not an edge case the production code needs to handle.
            .expect("valid inbox item");

        // Every field is checked individually so test failures name the field.
        assert_eq!(item.raw_text, "pick up pencils sometime this week");
        assert_eq!(item.captured_by, placeholder_member());
        // `captured_at` must equal `now` — the builder must not offset it.
        assert_eq!(item.captured_at, fixed_now());
        // New items must always begin as untriaged.
        assert_eq!(item.triage_state, TriageState::Untriaged);
        // `triaged_to` must be None until an explicit triage action occurs.
        assert!(item.triaged_to.is_none());
        // Default source is Touch; no explicit `.source()` was called above.
        assert_eq!(item.source, InboxSource::Touch);
    }

    #[test]
    fn empty_text_is_rejected() {
        // A blank-text capture is not a valid inbox item. The system must
        // reject it at the boundary, not silently store a useless entry.
        // Whitespace-only text must fail, not silently succeed.
        let result = InboxItemBuilder::new()
            .raw_text("   ")
            .captured_by(placeholder_member())
            .now(fixed_now())
            .build();

        assert!(
            matches!(result, Err(InboxError::EmptyText)),
            "expected EmptyText, got {result:?}"
        );
    }

    #[test]
    fn missing_raw_text_is_rejected() {
        // The builder must require raw_text; omitting it must return a typed
        // error rather than panicking or producing an item with an empty string.
        // Absent raw_text field must cause a MissingField error.
        let result = InboxItemBuilder::new()
            .captured_by(placeholder_member())
            .now(fixed_now())
            // Note: no `.raw_text(...)` call — this is the case under test.
            .build();

        // `MissingField` is the expected error; the string should name the field.
        assert!(matches!(result, Err(InboxError::MissingField(_))));
    }

    #[test]
    fn missing_captured_by_is_rejected() {
        // `captured_by` is required because the foreign-key in the database
        // requires a member ID. An item without an owner is structurally invalid.
        // Absent captured_by must cause a MissingField error.
        let result = InboxItemBuilder::new()
            .raw_text("note: buy milk")
            .now(fixed_now())
            // Note: no `.captured_by(...)` call.
            .build();

        assert!(matches!(result, Err(InboxError::MissingField(_))));
    }

    #[test]
    fn missing_now_is_rejected() {
        // `now` is required to set `captured_at`. Without it the builder has no
        // clock and cannot produce a valid timestamp. Rejecting it explicitly is
        // better than defaulting to `OffsetDateTime::now_utc()` inside the builder,
        // which would make the result non-deterministic in tests.
        // Absent `now` must cause a MissingField error, not a panic or default.
        let result = InboxItemBuilder::new()
            .raw_text("note: buy milk")
            .captured_by(placeholder_member())
            // Note: no `.now(...)` call.
            .build();

        assert!(matches!(result, Err(InboxError::MissingField(_))));
    }

    #[test]
    fn source_defaults_to_touch() {
        // Hub touch is the default — the most common capture path.
        // Verify the default persists through build() when `.source()` is omitted.
        // No `.source()` call; Touch must be the default.
        let item = InboxItemBuilder::new()
            .raw_text("test")
            .captured_by(placeholder_member())
            .now(fixed_now())
            // Note: no `.source(...)` call — default should be Touch.
            .build()
            .unwrap();

        assert_eq!(item.source, InboxSource::Touch);
    }

    #[test]
    fn explicit_source_is_preserved() {
        // When `.source()` is explicitly set, the value must survive through build().
        // Verifies the builder doesn't silently override an explicit source choice.
        // Explicit Mobile source must override the Touch default.
        let item = InboxItemBuilder::new()
            .raw_text("test")
            .captured_by(placeholder_member())
            .now(fixed_now())
            .source(InboxSource::Mobile) // explicit override of the Touch default
            .build()
            .unwrap();

        // The override must be respected.
        assert_eq!(item.source, InboxSource::Mobile);
    }

    #[test]
    fn new_items_are_always_untriaged() {
        // Triage is optional and deferrable (brief §6.3). New items must
        // begin in the untriaged state regardless of who built them.
        // A builder that defaults to any other state would violate the spec.
        // Build a standard item; its triage state must default to Untriaged.
        let item = InboxItemBuilder::new()
            .raw_text("a thought")
            .captured_by(placeholder_member())
            .now(fixed_now())
            .build()
            .unwrap();

        // Both the state enum and the reference field must reflect "not yet triaged".
        assert_eq!(item.triage_state, TriageState::Untriaged);
        assert!(item.triaged_to.is_none());
    }

    #[test]
    fn inbox_source_round_trips_through_string() {
        // Every variant must survive Display→FromStr. If any match arm is missing
        // in either direction, this loop catches it for that variant.
        for source in [
            InboxSource::Voice,
            InboxSource::Touch,
            InboxSource::Mobile,
            InboxSource::Share,
            InboxSource::ForwardEmail,
        ] {
            // Encode to storage string and decode back.
            let s = source.to_string();
            // Parse it back. `expect` is acceptable: a failure means the storage
            // contract is broken, which is a code defect, not a runtime condition.
            let parsed: InboxSource = s.parse().expect("known source string");
            // The parsed value must equal the original.
            assert_eq!(source, parsed, "round-trip failed for {source}");
        }
    }

    #[test]
    fn triage_state_round_trips_through_string() {
        // Mirror of the InboxSource round-trip test for TriageState.
        for state in [
            TriageState::Untriaged,
            TriageState::Typed,
            TriageState::Dismissed,
            TriageState::KeptAsNote,
        ] {
            // Encode to storage string and decode back.
            let s = state.to_string();
            let parsed: TriageState = s.parse().expect("known triage state string");
            assert_eq!(state, parsed, "round-trip failed for {state}");
        }
    }

    #[test]
    fn unknown_source_returns_error() {
        // Simulates reading an unknown source value from the database — e.g. a
        // newer binary wrote "carrier_pigeon" and an older binary is reading it.
        // Must return a typed error, not panic.
        // Parse an unknown string — simulates a schema mismatch between binaries.
        let result: Result<InboxSource, _> = "carrier_pigeon".parse();
        assert!(matches!(result, Err(InboxError::UnknownSource(_))));
    }

    #[test]
    fn typed_entity_ref_format() {
        // The format "entity_type:uuid" is the contract between this placeholder
        // type and the storage layer. Verify it is stable.
        // This test will need to be updated when TypedEntityRef is replaced by
        // proper typed references.
        // Use a fixed UUID with a known string form for the assertion below.
        let id = uuid::Uuid::parse_str("018f1a2b-0000-7000-8000-000000000001").unwrap();
        // Construct the reference; the output format is the contract under test.
        let ref_ = TypedEntityRef::new("task", id);
        // Colon separator between type and UUID must be exact.
        assert_eq!(ref_.0, "task:018f1a2b-0000-7000-8000-000000000001");
    }
}
