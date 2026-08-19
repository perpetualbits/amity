/* project-map.js — DATA for the Amity project map.
 *
 * This file is the single source of truth for the interactive project map
 * rendered by project-map.html. It is deliberately project-SPECIFIC: it
 * describes Amity, its architecture, and the honest status of each subsystem.
 * The renderer is project-agnostic and reads only `window.PROJECT_MAP`.
 *
 * Status is DERIVED, never invented:
 *   • The roadmap comes from the canonical brief §18.1 (Quarter-1 sketch) and
 *     §18.2 (validation drills).
 *   • Per-node status reflects what actually exists in the repository as of the
 *     `updated` date — verified by reading the crates, migrations, API router,
 *     and the Tauri frontend, not by trusting the task specs (which describe
 *     intended deliverables, some of which have not landed yet).
 *
 * Status vocabulary (see `statuses` below): done / active / seam / planned.
 * A "seam" is a deliberate architectural boundary that is wired but not yet
 * real — a placeholder, stub, or interface a future piece plugs into. Naming
 * seams honestly is the whole point: it keeps the map from claiming a green
 * light where there is only a connector waiting for wire.
 *
 * Keep this file in sync with the roadmap whenever the project's status changes.
 */

window.PROJECT_MAP = {
  // ── Project identity ──────────────────────────────────────────────────────
  project: {
    name: "amity",
    tagline: "a peaceful home — project map",
    repo: "github.com/perpetualbits/amity",
    updated: "2026-08-19",
  },

  // ── Status vocabulary ─────────────────────────────────────────────────────
  // Colours for these statuses are borrowed from Amity's own ambient-LED
  // language (brief §11.6): warm-white "settled", amber "attention", dim
  // "nothing pending". There is no red and no all-clear green, by design —
  // the product refuses both, so its map does too.
  statuses: {
    done: {
      label: "shipped",
      hint: "in the repository and covered by tests — it holds cleanly.",
    },
    active: {
      label: "in progress",
      hint: "partly shipped; a defining slice exists, more is on the bench.",
    },
    seam: {
      label: "seam",
      hint: "a deliberate boundary: wired and stubbed, waiting for the real piece.",
    },
    planned: {
      label: "planned",
      hint: "specified in the brief, not yet started. Nothing pending — quietly.",
    },
  },

  // ── Architectural bands, drawn top → bottom ───────────────────────────────
  layers: [
    { id: "surfaces", label: "Surfaces", hint: "what the household actually touches" },
    { id: "flows", label: "Flows", hint: "the two cross-cutting promises: capture and surfacing" },
    { id: "domain", label: "Domain", hint: "amity-core — typed entities, no I/O" },
    { id: "service", label: "Service", hint: "amity-service — the axum HTTP layer and its jobs" },
    { id: "foundation", label: "Foundation", hint: "storage, the home node, and the commitments beneath" },
  ],

  // ── Nodes ─────────────────────────────────────────────────────────────────
  nodes: [
    // ---- Surfaces ----------------------------------------------------------
    {
      id: "hub",
      label: "Hub app",
      layer: "surfaces",
      status: "active",
      tags: ["Tauri", "Solid", "Phase 1", "Task 3"],
      desc:
        "The kitchen hub — a Tauri 2 shell with a Solid frontend. Two views sit " +
        "behind a segmented control: Capture (the inbox) and Today (the ranked " +
        "surface, with mark-done / reassign and an inline task-capture form). " +
        "These type-check and bundle, but have not been run live — hub-tauri is " +
        "outside the Cargo workspace and needs WebKit2GTK. The hub-at-rest design " +
        "(clock, weather, status patch, ambient LED) is still to come, so the " +
        "surface as a whole stays in progress.",
      files: [
        "apps/hub-tauri/src/App.tsx",
        "apps/hub-tauri/src/Today.tsx",
        "apps/hub-tauri/src/Capture.tsx",
        "apps/hub-tauri/src/api.ts",
        "apps/hub-tauri/src-tauri/src/lib.rs",
      ],
      specs: [
        { label: "Task 1 — scaffolding & inbox", href: "docs/task_1_scaffolding_and_inbox.md" },
        { label: "Task 3 — surfacing & Today view", href: "docs/task_3_surfacing_and_today_view.md" },
        { label: "Brief §12 — voice & hub UX", href: "docs/amity_brief.md" },
      ],
      parts: [
        { label: "Inbox capture form", status: "done", desc: "Text input + submit; clears on success, no toast." },
        { label: "Recent-items list", status: "done", desc: "Refreshes on mount and after each capture; designed empty state." },
        { label: "Capture / Today switch", status: "done", desc: "A two-item segmented control; no router." },
        { label: "Today view", status: "done", desc: "Renders /surfacing/today as a mixed task+event list with a per-item kind marker (○ task / ◆ event); events show \"all day\" and carry no actions. Mark-done, reassign, and a calm empty state (type-checked, not yet run live)." },
        { label: "Task capture form", status: "done", desc: "Title, notes, due, tags, and a recurrence preset that builds the RRULE (not yet run live)." },
        { label: "Week view (UI)", status: "planned", desc: "BLOCKED, not built: the hub's native (Tauri) side does not compile on this machine — tauri-macros 2.6.3 (latest) trips E0255 on every rustc 1.88–1.95. Week backend + /week are ready; the SolidJS Week grid + live bring-up wait on an upstream tauri fix (or a full tauri-* family downgrade)." },
        { label: "Hub-at-rest (clock / weather / LED)", status: "planned", desc: "The calm screensaver baseline from brief §11.5–11.6." },
      ],
      deps: ["api"],
    },
    {
      id: "mobile",
      label: "Mobile companion",
      layer: "surfaces",
      status: "planned",
      tags: ["MVP", "Open Q"],
      desc:
        "A phone companion for quick-capture and checking off lists on the go. " +
        "The native-vs-PWA choice is still an open question (brief §19); the " +
        "clean service boundary keeps either option open.",
      files: [],
      specs: [
        { label: "Brief §5 — scope for MVP", href: "docs/amity_brief.md" },
        { label: "Brief §19 — open questions", href: "docs/amity_brief.md" },
      ],
      parts: [],
      deps: ["api"],
    },
    {
      id: "voice",
      label: "Voice intents",
      layer: "surfaces",
      status: "planned",
      tags: ["MVP", "push-to-talk"],
      desc:
        "Ten push-to-talk voice intents (add to groceries, add task, note, mark " +
        "done…). On-device transcription, audio discarded after parsing, no " +
        "always-on microphone. When an utterance is ambiguous it falls through " +
        "to an inbox note rather than failing silently.",
      files: [],
      specs: [{ label: "Brief §12 — voice & hub UX", href: "docs/amity_brief.md" }],
      parts: [],
      deps: ["inbox-flow", "api"],
    },
    {
      id: "kidmode",
      label: "Kid Mode",
      layer: "surfaces",
      status: "planned",
      tags: ["MVP"],
      desc:
        "A per-child, calm, age-appropriate Today view drawn from the same data " +
        "— large tappable items, plain language, no gamification of any kind. A " +
        "view, not a separate app; NFC / PIN / emoji switches context.",
      files: [],
      specs: [{ label: "Brief §10.5 — Kid Mode", href: "docs/amity_brief.md" }],
      parts: [],
      deps: ["surfacing"],
    },

    // ---- Flows -------------------------------------------------------------
    {
      id: "inbox-flow",
      label: "Unified inbox",
      layer: "flows",
      status: "done",
      tags: ["Phase 1", "capture"],
      desc:
        "The first promise Amity makes: one free-form intake for any thought. " +
        "Capture and recent-list are wired end-to-end (hub → service → storage) " +
        "and covered by tests. Triage into a typed entity is optional and " +
        "deferrable; the system never pesters for categorisation.",
      files: [
        "crates/amity-service/src/api/inbox.rs",
        "crates/amity-core/src/inbox.rs",
        "crates/amity-storage/src/inbox.rs",
      ],
      specs: [
        { label: "Brief §6.3 — inbox", href: "docs/amity_brief.md" },
        { label: "Task 1 — scaffolding & inbox", href: "docs/task_1_scaffolding_and_inbox.md" },
      ],
      parts: [
        { label: "Capture endpoint (POST /inbox)", status: "done", desc: "Generates id + timestamp, inserts, returns the item." },
        { label: "Recent query (GET /inbox/recent)", status: "done", desc: "N most-recent items; default 20, capped 100." },
        { label: "Non-touch sources", status: "seam", desc: "Voice / mobile / share / forward-email exist in the enum; only touch is wired." },
        { label: "Triage to typed entity", status: "seam", desc: "Task and Event both exist as triage targets now; Project is planned. TypedEntityRef is still a \"type:uuid\" string placeholder — the seams are drawn, the flow isn't built." },
      ],
      // Triage edges are typed "seam" dependencies: the inbox does not depend on
      // these entities, it *becomes* one when triaged. Each renders as a dashed,
      // labelled seam rather than a solid dependency line.
      deps: [
        "inbox-entity",
        "api",
        { to: "task", kind: "seam", label: "triage" },
        { to: "event", kind: "seam", label: "triage" },
        { to: "project", kind: "seam", label: "triage" },
      ],
    },
    {
      id: "surfacing",
      label: "Surfacing stream",
      layer: "flows",
      status: "active",
      tags: ["Phase 1", "Task 3", "Today / Week"],
      desc:
        "The hard part of the whole system: one ranked query that feeds the Today " +
        "view without becoming a nag-machine. The pure ranking rule and the " +
        "GET /surfacing/today endpoint are shipped and tested. It is now genuinely " +
        "mixed-type: Tasks and Events flow through one kind-agnostic rule — a task " +
        "surfaces on its day or when open-and-overdue, an event on its start day " +
        "(never overdue), all-day items lead the day. An empty day returns a calm " +
        "\"nothing today\". Project and Thread, and the Week view, are still to " +
        "come. Tone is a property of this layer: overdue reads as information, " +
        "never a lateness count.",
      files: [
        "crates/amity-core/src/surfacing.rs",
        "crates/amity-service/src/api/surfacing.rs",
      ],
      specs: [
        { label: "Task 3 — surfacing & Today view", href: "docs/task_3_surfacing_and_today_view.md" },
        { label: "Task 4 — events on Today", href: "docs/task_4_event_and_calendar.md" },
        { label: "Brief §6.4 — surfacing", href: "docs/amity_brief.md" },
        { label: "Brief §18 — roadmap", href: "docs/amity_brief.md" },
      ],
      parts: [
        { label: "Ranking rule (rank_today)", status: "done", desc: "Pure, tested, kind-agnostic: window ∩ today or overdue-open; all-day first, then time, then priority." },
        { label: "GET /surfacing/today endpoint", status: "done", desc: "Assembles task + event candidates from storage and returns uniform SurfacedItems + empty-state flag." },
        { label: "Empty-state result (has_surfaced)", status: "done", desc: "\"nothing today\" is a real result, not an error — brief §3." },
        { label: "Cross-entity ranking", status: "active", desc: "Task and Event now rank together through the kind-agnostic rule; Project and Thread still to come." },
        { label: "Today view", status: "done", desc: "The hub renders the mixed stream with a per-item kind marker (type-checked; live run pending WebKit)." },
        { label: "Week view", status: "active", desc: "Backend shipped (Task 6): pure plan_week planner (Mon-start 7-day buckets, overrides applied, open tasks only) + GET /week endpoint. The hub UI is blocked — see the hub node." },
      ],
      deps: ["task", "event", "api"],
    },
    {
      id: "notifications",
      label: "Notifications",
      layer: "flows",
      status: "planned",
      tags: ["MVP", "Phase 5"],
      desc:
        "The three-level attention model — Critical / Today / Soft — with a " +
        "near-silent default. Repeat once, never auto-escalate to a second " +
        "person. Drives the ambient LED (amber for an unacknowledged Critical; " +
        "no red, ever). None of this is built yet.",
      files: [],
      specs: [{ label: "Brief §11 — notifications & attention", href: "docs/amity_brief.md" }],
      parts: [],
      deps: ["surfacing"],
    },

    // ---- Domain (amity-core) ----------------------------------------------
    {
      id: "inbox-entity",
      label: "InboxItem",
      layer: "domain",
      status: "done",
      tags: ["Phase 1"],
      desc:
        "The domain type behind the inbox — raw text, who captured it, when, " +
        "from which source, and its triage state. A builder enforces the " +
        "construction invariants (non-empty text; captured-at not in the future).",
      files: ["crates/amity-core/src/inbox.rs", "crates/amity-core/src/ids.rs"],
      specs: [{ label: "Brief §6.3 — inbox", href: "docs/amity_brief.md" }],
      parts: [
        { label: "InboxItem + builder", status: "done", desc: "Validated construction; serde + Debug/Clone/PartialEq." },
        { label: "InboxSource / TriageState enums", status: "done", desc: "Voice/Touch/Mobile/Share/ForwardEmail; Untriaged/Typed/Dismissed/KeptAsNote." },
        { label: "TypedEntityRef", status: "seam", desc: "A \"type:uuid\" string placeholder until real typed refs exist." },
      ],
      deps: [],
    },
    {
      id: "task",
      label: "Task",
      layer: "domain",
      status: "done",
      tags: ["Phase 4", "chores"],
      desc:
        "The most-used entity in the system. Title, status, a due/earliest " +
        "window (not a point in time), effort, priority, tags, owner and " +
        "assignees, and an optional recurrence rule. The live fields are shipped " +
        "and tested; several columns are deliberately reserved for later tasks.",
      files: ["crates/amity-core/src/task.rs"],
      specs: [
        { label: "Brief §6.5 — Task", href: "docs/amity_brief.md" },
        { label: "Brief §8 — chores", href: "docs/amity_brief.md" },
        { label: "ADR-0003 — deferred Task fields", href: "docs/adrs/0003-task-fields-deferred.md" },
      ],
      parts: [
        { label: "Core fields + builder", status: "done", desc: "Trimmed non-empty title, window, assignees, tags; validated." },
        { label: "Status / Effort / Priority", status: "done", desc: "Open/Doing/Done/Skipped; Effort & Priority constrained 1..=5." },
        { label: "project_id", status: "seam", desc: "Nullable column, always NULL until the Project entity lands." },
        { label: "requires_ack", status: "seam", desc: "Reserved; depends on real member management." },
        { label: "checklist", status: "planned", desc: "Its own concern; a follow-up task." },
        { label: "attachments", status: "planned", desc: "Touches the storage subsystem; deserves its own task." },
      ],
      deps: ["recurrence"],
    },
    {
      id: "recurrence",
      label: "Recurrence engine",
      layer: "domain",
      status: "done",
      tags: ["Phase 3", "RRULE"],
      desc:
        "The architecturally novel piece, reusable by every entity that " +
        "recurs. An RRULE subset (RFC 5545) via the rrule crate, with validation " +
        "that rejects out-of-scope features with a clear error, and a " +
        "materialiser that anchors instances to local wall-clock time so DST " +
        "transitions don't move them.",
      files: [
        "crates/amity-core/src/recurrence.rs",
        "crates/amity-core/src/recurrence_materialiser.rs",
      ],
      specs: [
        { label: "ADR-0002 — recurrence engine", href: "docs/adrs/0002-recurrence-engine.md" },
        { label: "Brief §6.6 — time & recurrence", href: "docs/amity_brief.md" },
      ],
      parts: [
        { label: "Subset validation", status: "done", desc: "Rejects BYWEEKNO/BYYEARDAY/BYSETPOS/sub-daily FREQ with a 422-ready error." },
        { label: "DST-anchored materialiser", status: "done", desc: "60-day horizon; 08:00 stays 08:00 across DST — tested both directions." },
        { label: "Positional BYDAY", status: "done", desc: "1MO / -1FR for \"first Monday\", \"last Friday\"." },
        { label: "Holiday-skip hook", status: "seam", desc: "Accepts a skip-date set; the stub returns empty until NL holidays load." },
      ],
      deps: [],
    },
    {
      id: "completionlog",
      label: "CompletionLog",
      layer: "domain",
      status: "done",
      tags: ["Phase 4"],
      desc:
        "The visible record of who did what, and when. Marking a task done — or " +
        "skipped — writes a row here; skipping is just a kind of completion event " +
        "so the history stays uniform. The system never derives a fairness score " +
        "from it. Facts for humans to interpret, nothing more.",
      files: ["crates/amity-core/src/completion_log.rs", "crates/amity-storage/src/completion_log.rs"],
      specs: [
        { label: "Brief §8.2 — chore-completion model", href: "docs/amity_brief.md" },
        { label: "Brief §6.5 — CompletionLog", href: "docs/amity_brief.md" },
      ],
      parts: [],
      deps: ["task"],
    },
    {
      id: "people",
      label: "People & presence",
      layer: "domain",
      status: "seam",
      tags: ["placeholder"],
      desc:
        "Every task and inbox item references a member — but only one member " +
        "exists: a single hard-coded placeholder (00000000-…-0001) inserted at " +
        "migration time. The MemberId type is real; member management, two-tier " +
        "governance, and the Presence concept are all deferred to later tasks.",
      files: ["crates/amity-core/src/ids.rs", "crates/amity-storage/migrations/0001_initial.sql"],
      specs: [
        { label: "Brief §6.5 — Presence", href: "docs/amity_brief.md" },
        { label: "Brief §14.3 — two-tier governance", href: "docs/amity_brief.md" },
      ],
      parts: [
        { label: "MemberId newtype", status: "done", desc: "Typed UUID; usable now even without the Member entity." },
        { label: "Placeholder member", status: "seam", desc: "One hard-coded UUID backs every FK so the prototype runs." },
        { label: "Member management & governance", status: "planned", desc: "Creation, switching, PIN/NFC, admin/member tiers." },
        { label: "Presence", status: "planned", desc: "Home/away/with-other-parent windows that affect scheduling." },
      ],
      deps: [],
    },
    {
      id: "event",
      label: "Event & calendar",
      layer: "domain",
      status: "active",
      tags: ["MVP", "Phase 3", "Task 4", "Task 5"],
      desc:
        "Amity as a calendar aggregator, not a source of truth. The hub-native " +
        "half is shipped end-to-end: an Event entity (recurring or one-shot, " +
        "all-day or timed) reusing the recurrence engine, an EventOverride for " +
        "local overlays on an instance, storage, a create/list/get API, and " +
        "surfacing onto Today — an event shows on its start day, never overdue, " +
        "and a Cancel override removes it. The aggregator heart — read-only ICS " +
        "feeds (school, clubs, afvalkalender, NL holidays, personal calendars), " +
        "fetched under ADR-0004's egress guards and synced every ~6h — has now " +
        "shipped too (Task 5). Applying Reschedule/Annotate overrides to any " +
        "instance, native or external, remains the one open seam.",
      files: [
        "crates/amity-core/src/event.rs",
        "crates/amity-core/src/event_override.rs",
        "crates/amity-core/src/calendar.rs",
        "crates/amity-core/src/ics.rs",
        "crates/amity-storage/src/event.rs",
        "crates/amity-storage/src/calendar.rs",
        "crates/amity-service/src/api/event.rs",
        "crates/amity-service/src/api/calendar.rs",
        "crates/amity-service/src/feeds.rs",
        "crates/amity-service/src/jobs/calendar_sync.rs",
      ],
      specs: [
        { label: "Task 4 — event & calendar aggregation", href: "docs/task_4_event_and_calendar.md" },
        { label: "Task 5 — ICS ingestion design", href: "docs/superpowers/specs/2026-07-26-task-5-ics-ingestion-design.md" },
        { label: "ADR-0004 — external calendar ingestion", href: "docs/adrs/0004-external-calendar-ingestion.md" },
        { label: "Brief §6.5 — Event / EventOverride", href: "docs/amity_brief.md" },
        { label: "Brief §7 — calendars & time", href: "docs/amity_brief.md" },
      ],
      parts: [
        { label: "Event + EventSource + builder", status: "done", desc: "Native/ICS source, all-day or timed, optional recurrence; validated construction." },
        { label: "EventOverride", status: "done", desc: "Cancel / Reschedule / Annotate on one instance date; recorded end-to-end." },
        { label: "Storage (migration 0003)", status: "done", desc: "events + event_instances + event_overrides; source flattened onto the row." },
        { label: "Event API", status: "done", desc: "POST/GET /events, GET /events/{id}, POST /events/{id}/override — integration-tested." },
        { label: "Surfacing + Cancel override", status: "done", desc: "Events surface on Today via the kind-agnostic rule; a Cancel override removes the day's instance." },
        { label: "Reschedule / Annotate applied", status: "done", desc: "All three override actions now affect surfacing (Task 6 slice 1): Cancel hides the instance, Reschedule moves its time, Annotate attaches a note — on the shared instance path, external events included." },
        { label: "ICS ingestion & external feeds", status: "done", desc: "Read-only school/club/afvalkalender/holiday/personal calendars fetched, parsed, and synced every ~6h under ADR-0004's egress guards — the aggregator half, Task 5." },
      ],
      deps: ["recurrence"],
    },
    {
      id: "meals",
      label: "Meals, groceries, pantry",
      layer: "domain",
      status: "planned",
      tags: ["MVP", "Phase 2"],
      desc:
        "The one place real automation pays for itself: weekly dinner planning " +
        "(labels by default, recipes optional) feeding a grocery list, plus " +
        "coarse pantry levels and a use-first list. No calorie tracking, no " +
        "expiry alerts, no substitution engine. Not started.",
      files: [],
      specs: [{ label: "Brief §9 — meals, groceries, pantry", href: "docs/amity_brief.md" }],
      parts: [],
      deps: [],
    },
    {
      id: "project",
      label: "Projects, threads, anchors",
      layer: "domain",
      status: "planned",
      tags: ["MVP"],
      desc:
        "Soft-membership Projects (archiving one never kills its tasks), " +
        "relational Threads with no completion state (only \"touched base\", no " +
        "overdue red), and declared-only Anchors — standing claims on time that " +
        "are explicitly not obligations. Not started.",
      files: [],
      specs: [{ label: "Brief §6.5 — Project / Thread / Anchor", href: "docs/amity_brief.md" }],
      parts: [],
      deps: ["task"],
    },
    {
      id: "dietary",
      label: "Dietary profiles",
      layer: "domain",
      status: "planned",
      tags: ["MVP"],
      desc:
        "Safety flags (allergies) that produce acknowledgement-required warnings " +
        "and preference flags that produce passive notices — for household " +
        "members and recurring guests alike. Neither ever hard-blocks a meal. " +
        "Not started.",
      files: [],
      specs: [{ label: "Brief §9.3 — dietary needs", href: "docs/amity_brief.md" }],
      parts: [],
      deps: ["meals"],
    },

    // ---- Service (amity-service) ------------------------------------------
    {
      id: "api",
      label: "HTTP API",
      layer: "service",
      status: "active",
      tags: ["axum", "/api/v1"],
      desc:
        "The axum router under /api/v1. The inbox endpoints (2), the full task " +
        "surface (9, including complete / skip / assignee / history), the " +
        "event endpoints (4: create / list / get / override), and the calendars " +
        "endpoints (6: subscribe / list / get / enable-disable / delete / refresh) " +
        "are all wired and tested. Auth is deliberately absent — the service " +
        "binds to loopback only for now; it grows as more entities land.",
      files: ["crates/amity-service/src/api", "crates/amity-service/src/lib.rs"],
      specs: [
        { label: "Task 1 — API conventions", href: "docs/task_1_scaffolding_and_inbox.md" },
        { label: "Task 2 — Task API", href: "docs/task_2_task_entity.md" },
        { label: "Task 4 — Event API", href: "docs/task_4_event_and_calendar.md" },
        { label: "Task 5 — Calendars API", href: "docs/superpowers/specs/2026-07-26-task-5-ics-ingestion-design.md" },
      ],
      parts: [
        { label: "Inbox endpoints", status: "done", desc: "POST /inbox, GET /inbox/recent." },
        { label: "Task endpoints", status: "done", desc: "CRUD + complete / skip / assignee / upcoming / history." },
        { label: "Event endpoints", status: "done", desc: "POST/GET /events, GET /events/{id}, POST /events/{id}/override." },
        { label: "Calendars endpoints", status: "done", desc: "POST/GET /calendars, GET /calendars/{id}, PATCH /calendars/{id}, DELETE /calendars/{id}, POST /calendars/{id}/refresh." },
        { label: "Auth", status: "planned", desc: "Loopback-only isolation today; real auth arrives with members." },
        { label: "Remaining entities", status: "planned", desc: "Meal, Project… each adds a handler module." },
      ],
      deps: ["storage"],
    },
    {
      id: "jobs",
      label: "Recurrence horizon job",
      layer: "service",
      status: "done",
      tags: ["Phase 4", "Task 3"],
      desc:
        "Recurring tasks materialise their instances at creation time, out to a " +
        "60-day horizon. The background job that rolls that horizon forward and " +
        "prunes aged-out instances — the ADR-0002 TODO — now exists: run_once does " +
        "one maintenance pass (materialise every recurring task to +60 days, prune " +
        "instances older than 30 days), and spawn runs it on startup and daily via " +
        "a tokio interval. The materialisation mapping is shared with the task-create " +
        "path so it lives in one place.",
      files: [
        "crates/amity-service/src/jobs/recurrence_horizon.rs",
        "crates/amity-storage/src/task_instance.rs",
      ],
      specs: [{ label: "ADR-0002 — recurrence engine", href: "docs/adrs/0002-recurrence-engine.md" }],
      parts: [
        { label: "Materialise-at-create", status: "done", desc: "New recurring tasks get instances up to the 60-day horizon immediately." },
        { label: "Prune helper", status: "done", desc: "prune_old_instances in storage keeps the last 30 days." },
        { label: "Daily horizon extension", status: "done", desc: "run_once + spawn roll the horizon forward and prune on startup and every 24h." },
      ],
      deps: ["recurrence", "storage"],
    },
    {
      id: "config",
      label: "Config & startup",
      layer: "service",
      status: "done",
      tags: ["Phase 1"],
      desc:
        "TOML configuration loaded from the platform config directory with " +
        "sensible built-in defaults, structured logging via tracing, and the " +
        "data-directory auto-create that stops the service crashing on first " +
        "run (the punch-list fix from Task 2).",
      files: ["crates/amity-service/src/config.rs", "crates/amity-service/src/main.rs"],
      specs: [
        { label: "Task 1 — config format & location", href: "docs/task_1_scaffolding_and_inbox.md" },
        { label: "Task 2 — data-dir auto-create", href: "docs/task_2_task_entity.md" },
      ],
      parts: [],
      deps: [],
    },

    // ---- Foundation --------------------------------------------------------
    {
      id: "storage",
      label: "Storage & migrations",
      layer: "foundation",
      status: "done",
      tags: ["Phase 1", "sqlx", "SQLite"],
      desc:
        "sqlx over SQLite with embedded migrations that apply automatically on " +
        "first run. Four migrations so far (inbox; tasks + instances + completion " +
        "logs; events + instances + overrides; calendars + the unique index that " +
        "makes external-event upsert well-defined) and a repository module per " +
        "entity. ISO-8601 datetimes and TEXT UUIDs keep the schema portable to " +
        "Postgres later.",
      files: ["crates/amity-storage/src", "crates/amity-storage/migrations"],
      specs: [
        { label: "ADR-0001 — initial architecture", href: "docs/adrs/0001-initial-architecture.md" },
        { label: "Task 1 — storage layer", href: "docs/task_1_scaffolding_and_inbox.md" },
        { label: "ADR-0004 — external calendar ingestion", href: "docs/adrs/0004-external-calendar-ingestion.md" },
      ],
      parts: [
        { label: "Connection / pool", status: "done", desc: "Pool setup; migrations embedded at compile time." },
        { label: "Inbox repository", status: "done", desc: "insert / fetch / list-recent, with an index on captured_at." },
        { label: "Task / instance / log repositories", status: "done", desc: "Full insert-materialise-complete cycle, integration-tested." },
        { label: "Event / instance / override repositories", status: "done", desc: "Event source flattened onto the row; instances upserted; overrides looked up by date. Integration-tested." },
        { label: "Calendars repository (migration 0004)", status: "done", desc: "calendars table + sync state; insert/list/fetch/update/delete, integration-tested." },
        { label: "External-event upsert / prune", status: "done", desc: "upsert_external_events (ON CONFLICT on source_calendar_id + source_external_id) and prune_events_missing_from_feed keep a feed's events in sync without wiping on a transient fetch failure." },
        { label: "Postgres portability", status: "planned", desc: "Kept portable by convention; not exercised until there's a reason." },
      ],
      deps: [],
    },
    {
      id: "homenode",
      label: "Home node platform",
      layer: "foundation",
      status: "planned",
      tags: ["direction"],
      desc:
        "Local-first by architecture, not by promise. MVP runs on a kiosk " +
        "tablet; the design is meant to migrate to a small always-on home server " +
        "(1TB, low-power, UPS) without a rewrite. The planner is the first " +
        "application; the home node is the platform. Architectural direction, " +
        "not yet a build target.",
      files: [],
      specs: [
        { label: "Brief §15 — hardware & the home node", href: "docs/amity_brief.md" },
        { label: "Brief §17 — long-term direction", href: "docs/amity_brief.md" },
      ],
      parts: [],
      deps: [],
    },
    {
      id: "privacy",
      label: "Privacy & governance",
      layer: "foundation",
      status: "seam",
      tags: ["categorical"],
      desc:
        "The categorical commitments — no surveillance vectors, no commercial " +
        "data flow, local-first — are already honoured by the architecture: the " +
        "service itself is loopback-only, and the one exception — Task 5's " +
        "outbound ICS feed fetches — is a user-initiated or user-scheduled " +
        "read-only GET of a calendar the household chose, carrying no household " +
        "data, bounded by ADR-0004's egress guards. The user-facing governance " +
        "(two tiers, per-item visibility with no admin reveal, the 90-day change " +
        "log, the help-finding affordance) is not built yet — it arrives with " +
        "real members.",
      files: ["docs/amity_philosophy.md", "docs/adrs/0004-external-calendar-ingestion.md"],
      specs: [
        { label: "Brief §2 — categorical commitments", href: "docs/amity_brief.md" },
        { label: "Brief §14 — privacy, governance, safety", href: "docs/amity_brief.md" },
        { label: "ADR-0004 — external calendar ingestion", href: "docs/adrs/0004-external-calendar-ingestion.md" },
      ],
      parts: [
        { label: "Local-first / loopback-only", status: "done", desc: "The service does not listen beyond localhost; all data is created and read on the device." },
        { label: "Categorical commitments", status: "done", desc: "Enforced by architecture — no telemetry, no always-on capture, no commercial data flow. Outbound traffic is limited to user-initiated/scheduled read-only calendar-feed fetches (see below), never surveillance or profiling." },
        { label: "ICS fetch egress guards (ADR-0004)", status: "done", desc: "Task 5's outbound fetch — the system's first — is bounded by a scheme allow-list (http/https only), a 20s timeout, a 5 MiB cap enforced while streaming, a 5-redirect bound, and no compression compiled in; it sends no household data, only a plain GET of the configured feed URL. Each guard (size cap incl. the at-cap boundary, non-2xx, redirect bound) is exercised by HTTP-level tests against a local mock server — crates/amity-service/tests/feeds_egress.rs." },
        { label: "Two-tier governance", status: "planned", desc: "Admin / member, with admins content-blind to private items." },
        { label: "Per-item privacy / no admin reveal", status: "planned", desc: "Owner-set visibility; no override exposes a child's private item." },
        { label: "Change log & audit", status: "planned", desc: "Factual 90-day record; no blame view, no monitoring." },
      ],
      deps: ["people"],
    },
    {
      id: "sync",
      label: "Cloud sync (E2E)",
      layer: "foundation",
      status: "planned",
      tags: ["post-MVP"],
      desc:
        "Opt-in, end-to-end encrypted sync where the cloud party holds only " +
        "ciphertext — post-MVP by design. Distributed peer-to-peer backup " +
        "(I2P / Tahoe-LAFS) is an explicit long-term candidate the current " +
        "architecture must not foreclose. Not started.",
      files: [],
      specs: [
        { label: "Brief §14.1 — data & deletion", href: "docs/amity_brief.md" },
        { label: "Brief §17 — long-term direction", href: "docs/amity_brief.md" },
      ],
      parts: [],
      deps: ["storage"],
    },
  ],

  // ── Roadmap ────────────────────────────────────────────────────────────────
  // A "next" entry names the actively-scheduled next task (does not count toward
  // the phase/hardening tallies). Phases come from the brief's Quarter-1 sketch
  // (§18.1); hardening items from the validation drills (§18.2). Status reflects
  // reality, which diverged from the
  // linear plan: the maintainer built vertically — foundation plus the Task
  // backend and its recurrence engine — rather than strictly phase by phase, so
  // phases 1, 3 and 4 are all partly done at once while 2, 5 and 6 are untouched.
  // Task 6 shipped its backend (Reschedule/Annotate overrides + the Week-view
  // planner and /week endpoint). Its hub half — the live prototype and the Week
  // UI — is blocked by an upstream tauri-macros/E0255 build failure, so the
  // "next" entry names that follow-up rather than Task 7.
  roadmap: [
    { id: "t6b", kind: "next", label: "Task 6 follow-up · hub live + Week UI (blocked: tauri-macros E0255)", status: "planned" },
    { id: "p1", kind: "phase", label: "P1 · Data model + inbox + Today", status: "active" },
    { id: "p2", kind: "phase", label: "P2 · Meals, lists, pantry", status: "planned" },
    { id: "p3", kind: "phase", label: "P3 · Calendar + recurrence engine", status: "active" },
    { id: "p4", kind: "phase", label: "P4 · Tasks + CompletionLog + chores", status: "active" },
    { id: "p5", kind: "phase", label: "P5 · Notifications + hub-at-rest + LED", status: "planned" },
    { id: "p6", kind: "phase", label: "P6 · Polish + accessibility + pilot", status: "planned" },
    { id: "h1", kind: "harden", label: "Paper-prototype the four views", status: "planned" },
    { id: "h2", kind: "harden", label: "Cognitive walkthrough (5 tasks)", status: "planned" },
    { id: "h3", kind: "harden", label: "Privacy drill — no admin-reveal path", status: "planned" },
    { id: "h4", kind: "harden", label: "Data-export rehearsal (GDPR)", status: "planned" },
    { id: "h5", kind: "harden", label: "Offline / outage drill", status: "planned" },
  ],
};
