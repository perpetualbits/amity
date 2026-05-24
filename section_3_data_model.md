## 3) Core Concepts & Data Model

*Replacement for the original §3. Reflects discussion of: aggregator-style calendar (hub owns family events; externals are read-only sources with local overrides), the "open loops" reframe (the system as a trustworthy resting place for things the mind would otherwise hold), and the principled separation of obligation-bearing entities from anchors (standing claims on time that the system serves on pull, not push).*

### 3.1 Premise

The system's primary job is to be a place where things can rest, so the people in the household can stop holding them. A grocery item, a friend not called in weeks, a chimney sweep, a child's homework, a vague intent to play the guitar — psychologically they are all *open loops*. The mind keeps them present at a cost. A trustworthy planner absorbs that cost; an untrustworthy one adds to it. Every decision in this model is judged against that single test.

The explicit failure modes this design refuses:

- Becoming another source of nagging or guilt.
- Pretending to mediate fairness, negotiation, or motivation between people.
- Producing engagement for its own sake (streaks, points, dopamine loops).
- Demanding categorisation at the moment of capture.

### 3.2 The Open Loop concept (conceptual layer)

"Open loop" is a *conceptual* property shared by several distinct entity types, not a single database table. Collapsing everything into one `OpenLoop` row with a `type` column produces a schema full of mostly-null fields and unqueryable JSON blobs; every system that tries it eventually re-grows the type-specific tables under uglier names. We keep type-specific entities for storage and query, and put the unified experience in two layers above them:

- **Capture (one inbox)** — any input arrives as a raw item. No type required.
- **Surfacing (one query)** — *Today* and *Week* views pull from all entity types and render a single mixed, ranked stream.

Both layers are described before the entities, because the entities are an implementation detail behind them.

### 3.3 Capture: the inbox

A single intake point, reachable from voice, hub touch, mobile quick-capture, and (later) mobile widget. The capture UI takes a free-form utterance and stores it as an `InboxItem`:

```
InboxItem {
  id, rawText, capturedBy, capturedAt, source(voice|touch|mobile|share),
  attachments[],            # optional photo, audio clip, link
  triagedTo,                # nullable: id of the typed entity it became
  triageState(enum: untriaged | typed | dismissed | kept_as_note)
}
```

Critical property: **triage is optional and deferrable.** Items may remain `untriaged` indefinitely. The system does not pester for categorisation. Users can leave a note as a note. The inbox is also itself surfacable — if a user wants to skim un-triaged items, there is a view for that, but the system does not push them to do so.

### 3.4 Surfacing: the unified stream

A single query produces the *Today* view and feeds the *Week* view. It returns a ranked, mixed-type stream drawn from `Event`, `Task`, `Project` milestones, and `Thread` soft-cadence prompts. Anchors do **not** appear here automatically.

Ranking inputs (not a formula — direction of influence only):

- Time proximity (event in 2 hours > task due tomorrow).
- Stated priority on the item itself.
- Per-person filtering (everyone's view; my view).
- Quiet hours and household rhythm (defined in §H, notifications).

Two properties of this layer matter more than the ranking maths:

**The "nothing to do" state is a real designed state.** When the ranked stream is below a threshold of worth-mentioning, the *Today* view does not scratch around for something to display. It says, clearly and calmly, that the household is caught up. This is unusual for productivity software and is intentional — a planner that always finds another nag is, over time, indistinguishable from the clutter it was meant to remove.

**Tone is a property of the layer, not of each item.** "Bin night tomorrow — Alex" is information. "Overdue: call mum — 3 weeks" is a guilt-trip wearing information's clothes. The surfacing layer is responsible for tone; entities just carry facts.

### 3.5 Entities

Each entity below carries cross-cutting fields defined once in §3.6. Only entity-specific fields are listed here.

#### Event *(MVP)*

Datetime-bound. The hub mostly displays events from external calendars (aggregator role, per §10 integrations). Family events created at the hub live in a native hub-owned calendar.

```
Event {
  title, startAt, endAt, allDay, location?, attendees[],
  reminders[],
  source { kind: native|google|apple|ics,
           externalId?, calendarId?,
           readOnly: bool, lastSyncedAt? }
}
```

The `source` field is essential. The UI uses it to decide what edit affordances to offer (full controls for native; "create override" only for read-only externals).

#### EventOverride *(MVP)*

Local overlay applied to instances of read-only external events. Lets the household record "bin day moved because of King's Day" without trying to write back to the municipality's ICS feed.

```
EventOverride {
  sourceEventId, instanceDate,
  action(enum: cancel | reschedule | annotate),
  payload,                # new datetime, note text, etc.
  createdBy, createdAt
}
```

Overrides are applied at display time. The underlying external event is never modified.

#### Presence *(MVP)*

Represents a member's availability state over a time window. Not a calendar event — a separate concept that *affects* how scheduling and rotation decisions are made. Covers business travel, offshore rotations, shared-custody alternation, illness, holidays.

```
Presence {
  memberId,
  state(enum: home | away | offshore | with_other_parent | traveling | unavailable),
  from, until,
  note?,
  affectsChoreRotation: bool   # default: true for away/offshore/with_other_parent
}
```

Behaviour:

- The chore rotation logic consults `Presence` before proposing assignees.
- The *Week* view visually mutes members who are not `home` for that day.
- "Who's at dinner Wednesday?" is answered by querying Presence, not by guessing from calendar events.
- For children under shared custody, alternating-week patterns are expressed as recurring Presence windows. This avoids needing custody-specific code paths elsewhere.

#### Task *(MVP)*

Due-by *window*, not a slot. Completable. May recur. Covers chores, errands, recurring obligations whose timing is flexible (taxes by April 30; clothes shopping before winter; chimney sweep this year).

```
Task {
  title, notes?, ownerId?, assigneeIds[],
  dueBy?, earliestAt?,    # window, not point-in-time
  effort?, priority?,
  status(enum: open | doing | done | skipped),
  recurrence?,            # rule string; see 3.7
  checklist[], attachments[],
  tags[]                  # free-form; replaces categories
}
```

Note the absence: no `fairness_score`, no `difficulty`. Fairness is a *computed view*, not stored on the task (per §C of the brief). Difficulty was in the original §3 but adds little — effort already captures what matters operationally.

#### Project *(MVP)*

Multi-step, long-horizon. Parent container for sub-tasks and milestones. Covers house repairs and build plans, garden master plan, vacation preparation, the seven-year repaint cycle.

```
Project {
  title, description?,
  status(enum: active | paused | done | archived),
  horizon?,               # rough timeframe; "spring 2027", optional
  milestones[ { title, targetDate?, doneAt? } ],
  childTaskIds[], childProjectIds[],
  attachments[], location?,
  ownerId?
}
```

Projects do not appear in the *Today* stream directly. Their *milestones* and *child tasks* do, when proximate.

#### Thread *(MVP)*

Relational, no completion state. A standing connection to a person or small group of people that wants occasional attention. Designed for social maintenance — the mental load of remembering who you haven't spoken to in a while.

```
Thread {
  title,                  # "Mum", "Henk & Marie", "Anneke"
  ownerId?,               # whose thread this primarily is
  desiredCadence?,        # "every ~2 weeks"; soft, not enforced
  lastTouchedAt?,
  channelHint?,           # "phone", "visit", "message"
  notes?
}
```

Threads have no "done" — they have *touched base*. The UI affordance is "touched base today", which updates `lastTouchedAt`. No completion percentage, no streak, no overdue red. The *Today* surfacing layer can include a Thread when `desiredCadence` has lapsed, but with gentle wording and a low rank; the user can also pull the Threads view to see them on their own time.

#### Anchor *(MVP — declared and pulled only; auto-surfacing is post-MVP)*

A standing claim on time and attention that is explicitly *not* an obligation. "Guitar". "Long walks alone". "Reading fiction in the bath". These are the things that make life worth living, in the user's own definition, and the system's job is to know they exist without policing them.

```
Anchor {
  id, ownerId,
  title,
  preferredWindows?,      # "weekday evenings"; optional, soft
  notes?,
  createdAt
}
```

MVP scope:

- Users can declare, edit, and delete anchors.
- A pull-only view ("what about me?") shows the owner their own anchors.
- The system does **not** surface anchors in *Today* automatically.
- No tracking of how often an anchor is "honoured". No completion. No streaks. Ever.

Post-MVP: the system may *offer* an anchor in genuinely empty time, with gentle wording, low frequency, and no nag-state. This is deferred because surfacing requires the obligation layer to be mature enough to reliably distinguish "you are caught up" from "you haven't entered everything yet" — surfacing anchors over noise is worse than not surfacing them at all.

### 3.6 Cross-cutting fields

Every entity carries:

- `id`, `createdBy`, `createdAt`, `updatedAt`.
- `visibility(enum: family | adults | private)` and per-member overrides (see §9 privacy).
- `ownerId?` — the member primarily responsible for the item. Distinct from `assigneeIds[]` on Task: owner is *accountability*, assignees are *doing*. They may be the same person.
- `tags[]` — free-form. The system does not impose a taxonomy.

### 3.7 Time zones and recurrence policy

These are policy defaults for the system. They can be overridden per-item but should rarely need to be.

**Time zones.** Each `Event` stores its time zone alongside its datetime. Native hub events default to Europe/Amsterdam. Externally-sourced events keep whatever the source declared.

- **The hub always displays home time** (Europe/Amsterdam). It is a kitchen device, in the kitchen. There is no "viewer time zone" question.
- **The mobile app displays home time by default**, with an optional secondary line showing "your current local time" when the user is traveling. This applies only to events the user is personally attending; place-anchored events (school, bins) show home time only.
- Events explicitly anchored to a place (school day, garbage collection) are never converted to a traveler's local time. They show as they are in the place where they happen.

**Recurrence edge defaults.**

- **Holidays.** For externally-sourced recurring events, trust the source — the municipality and the schools already handle Dutch public holidays in their ICS feeds. For native recurring Tasks and Events: skip on declared NL public holidays for items tagged `chore` or `household`; run regardless otherwise. The holiday list is loaded from a maintained public source at install and refreshed annually.
- **Daylight savings.** All native recurring items anchor to local time. "08:00 every Thursday" stays at 08:00 on the household's wall clock through DST transitions.
- **Deleting a single instance** of a recurring item creates an `EventOverride` with `action=cancel` for that instance only. An undo affordance offers "actually, delete the whole series" within a short window. Default action is the safe one (one instance), not the destructive one (whole series).

### 3.8 Recurrence and cadence (draft DSL)

Recurrence is expressed as a small human-readable string, parsed into a rule. Examples:

```
every=week; on=Mon
every=month; on=1st Sat
every=2 weeks; on=Thu
every=year; on=October
```

Cadence (used by Thread `desiredCadence` and post-MVP Anchor surfacing) is softer:

```
~2 weeks
~3 months
~yearly
```

The tilde is meaningful: cadence is a *preference*, not a rule. The system never raises an alarm when a cadence elapses; it just adjusts what it might gently surface.

### 3.9 Design boundaries — what this model is *not* trying to be

These are positive scope choices, not omissions.

- **Not a fairness arbiter.** Effort is stored; *fairness scores* are computed views shown to humans who decide for themselves. The system does not propose reassignment unprompted. (Aligned with §C of the brief.)
- **Not a motivation engine.** No streaks, no badges, no points, no engagement metrics. Reward comes from the felt experience of a calmer household, not from the software.
- **Not a CRM.** Threads exist for relational rhythm, not contact management. Phone numbers, addresses, birthdays belong elsewhere (phone, contacts app); a Thread holds *attention*, not *data*.
- **Not a goal-tracker.** Anchors are not goals. They are reminders the user issues to themselves of who they want to be. The system does not measure progress toward them.
- **Not an engagement product.** A successful day on the hub is one where the family barely needed to look at it. The product is allowed — encouraged — to recede.

### 3.10 What changed from the original §3

- Original `Recipe`, `Meal`, `List`, `MaintenanceItem` entities are deferred to their respective cluster discussions (Cluster 3 meals/groceries; Cluster 2 chores/maintenance) and not redefined here. They will hang off `Task` and `Project` where appropriate.
- `difficulty` field removed from Task (subsumed by `effort`).
- `RotationRule` is retained conceptually but moves to the Cluster 2 chores discussion. It is a behaviour over Tasks, not an entity.
- New entities: `InboxItem`, `EventOverride`, `Presence`, `Project`, `Thread`, `Anchor`.
- New explicit layers above entities: capture (inbox) and surfacing (unified stream).
- New time-zone and recurrence policy section.
- New explicit design-boundaries section.
