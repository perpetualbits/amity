# Amity

*A household at peace.*

---

*Canonical project brief, v2. Supersedes the v1 brief in its entirety. Reflects design work across eight thematic clusters; integrates the data model, design philosophy, and architectural commitments developed in that work.*

*Last updated: 2026-05-24.*

---

## Contents

1. [Vision and purpose](#1-vision-and-purpose)
2. [Categorical commitments](#2-categorical-commitments)
3. [Design principles](#3-design-principles)
4. [Personas and jobs-to-be-done](#4-personas-and-jobs-to-be-done)
5. [Scope for MVP](#5-scope-for-mvp)
6. [Data model](#6-data-model)
7. [Calendars and time semantics](#7-calendars-and-time-semantics)
8. [Chores and home maintenance](#8-chores-and-home-maintenance)
9. [Meals, groceries, pantry](#9-meals-groceries-pantry)
10. [Children, homework, Kid Mode](#10-children-homework-kid-mode)
11. [Notifications and attention](#11-notifications-and-attention)
12. [Voice and hub UX](#12-voice-and-hub-ux)
13. [Integrations](#13-integrations)
14. [Privacy, governance, safety](#14-privacy-governance-safety)
15. [Hardware and the home node](#15-hardware-and-the-home-node)
16. [Internationalization](#16-internationalization)
17. [Long-term direction](#17-long-term-direction)
18. [Roadmap and validation](#18-roadmap-and-validation)
19. [Open questions](#19-open-questions)

---

## 1. Vision and purpose

Amity helps a household share the cognitive load of home life. It replaces the kitchen chalkboard, the paper calendar, the scattered sticky notes, and — most importantly — the load of unspoken obligations carried in the head of whoever holds it most heavily, typically the primary care-giver.

The system is a *trustworthy resting place* for the things a mind would otherwise hold. A grocery item, a friend not called in weeks, a chimney sweep due, a child's homework, a vague intent to play the guitar — all of these are *open loops* that exert a small constant cost on the people who hold them. A planner that absorbs that cost gives back something the household didn't know was leaving: attention, calm, and the felt sense that things are under control.

The system's success is measured not by features used, engagement time, or tasks completed, but by whether the household feels lighter. A good day on Amity is one where the family barely needed to look at it. The product is allowed — encouraged — to recede.

This is what "a household at peace" means in concrete terms. Not a household with no conflict, no obligations, no friction — those are unrealistic and the system would be lying to promise them. A household at peace is one where the inevitable obligations of family life are *held cleanly*, where attention is freed from low-level vigilance, and where the relationships among the people in the home are not burdened by mental clutter that the software could absorb.

## 2. Categorical commitments

These are not configurable settings. They are project-defining boundaries. Future requests to soften, work around, or revisit them are rejected by definition. They exist because without them, this project would, over time, become indistinguishable from the things it was built to replace.

**The kitchen is a safe place.** The system never listens unless explicitly invoked. No always-on microphones, no wake-word hotword running continuously, no ambient audio capture. Push-to-talk only. No facial recognition, no biometric capture, no presence-detection via cameras. The same principle extends to visual capture: photo-based features are explicit, single-shot, and discarded after processing — never automatic.

**No surveillance vectors.** The system does not implement features whose purpose is for one party (commercial, governmental, or within the household) to track another's compliance, location, activity, or behaviour. This includes "monitoring" features that one household member might want over another, which the system simply does not provide.

**No commercial data flow.** No telemetry that exposes user behaviour to third parties. No advertising integration. No partnerships that involve household data in any direction. No data flow that could become an advertising surface for the receiving party. The household's data belongs to the household.

**Local-first by architecture, not by promise.** The household's data lives on the household's own hardware. Cloud sync is opt-in only and end-to-end encrypted such that the cloud party holds only ciphertext. The architecture makes the categorical commitments enforceable rather than merely declared.

**No mediation of human relationships.** The system does not arbitrate fairness between household members, does not manufacture motivation through gamification, does not supervise compliance, does not insert itself into the rewards humans give one another. Where these temptations are most enticing — chores, food, children, attention — the system's restraint is the feature.

These commitments operationalise into a checklist applied to any future proposed feature or integration:

- Does data flow outward? To whom, how much, when, what is the recourse?
- Could the data become an advertising surface for the receiving party?
- Does the feature require always-on capture of any kind?
- Does it create a commercial relationship that gives a third party leverage?
- Can the same value be delivered by user-initiated action instead?

If any of the first four answers raises concern, the feature is rejected. The checklist is the operationalisation of the commitments and belongs in the project's contributor documentation.

## 3. Design principles

The principles below shape decisions wherever the categorical commitments don't dictate them. They emerged from the design process; they describe what Amity *does* as opposed to what it *won't*.

**The system holds, displays, and gets out of the way.** It does not arbitrate, motivate, or supervise. Where humans should be in charge of a decision — whose turn it is, whether a child has done their homework well, whether a meal contains too much of something — the system makes the facts visible and steps back.

**Capture must be near-frictionless.** If putting a thought into Amity is harder than holding it in the head, Amity loses. Voice, tap, mobile quick-capture, and forwarding all converge on a single unified inbox. The user does not need to choose a category at capture time; triage is optional and deferrable.

**Surfacing is the hard part, not storing.** Any database can remember. The skill is showing the right thing at the right moment without becoming a nag-machine. Amity errs toward silence — the cost of under-notifying is concrete and bounded; the cost of over-notifying is existential to the project.

**The empty state is a designed state.** When nothing demands attention, the hub displays calm — a clock, the weather, a quiet "nothing today". A planner that always finds another nag is, over time, indistinguishable from the clutter it was meant to remove. Amity is allowed — required — to say there is nothing to do.

**Don't build features whose maintenance burden is borne by the user and sustained only by guilt.** Calorie tracking, expiry-date logging, water-intake reminders, streak metrics — these features work in demos and fail in week three because the cost of upkeep exceeds the value of the data. Amity does not build them.

**Structure grows with use.** The system never demands setup before it provides value. A household can use it for years with nothing but freeform meal labels and a manually-managed grocery list. Structured features (recipes, pantry thresholds, dietary profiles) are available when the user wants their payoff; they are never mandatory.

**Soft membership over strict ownership.** Tasks may belong to projects without being owned by them. Items may carry tags without being typed. Members may join household-wide threads or stay personal. The data model prefers loose coupling — a project archived doesn't kill the recurring chores under it; a tag changed doesn't break a workflow.

**Tone is a property of the system, not of each item.** Items carry facts ("bin night tomorrow — Alex"). The system's surfacing layer is responsible for *how* those facts feel ("overdue: call mum — 3 weeks" wears information's clothes but is a guilt-trip). The same fact, surfaced differently, becomes information or pressure. Amity chooses information.

## 4. Personas and jobs-to-be-done

**The primary care-giver** (often a parent, often the mother in current households, but Amity actively resists making this default): wants the cognitive load distributed and the system to absorb the mental list. Wants to trust that things will be remembered without her holding them.

**The other adult(s) in the household:** wants visibility into what's happening, the ability to take on more without ceremony, and a planning surface for longer-horizon projects (house repairs, garden, vacations).

**Teens:** want clear, bite-sized commitments and meaningful privacy. Want to be treated as people with their own emerging autonomy, not as objects in the parents' planning system.

**Children:** want calm, age-appropriate views of their day. Want their parents' attention to be present and unmediated, not surveilled-through-software.

**Guests with persistent needs:** want their dietary, accessibility, or other persistent considerations remembered between visits without re-explaining each time.

Concrete jobs-to-be-done include: planning a week's dinners and generating the shopping list; remembering to put out the bins; coordinating who picks up which child from which activity; tracking which household maintenance items are due this season; keeping track of homework without the homework becoming a parent–child compliance issue; remembering to maintain social connections; and, most fundamentally, *not having to remember*.

## 5. Scope for MVP

The MVP delivers the system's core value — a trustworthy resting place — for a single household, with the kitchen-hub touch interface and mobile companion apps. Beyond MVP, the system grows along the trajectory in section 17.

**In MVP:**

- Unified inbox capture (voice push-to-talk, hub tap, mobile quick-capture).
- The four core views — Today, Week, Meals, Lists — with the empty state and minimal hub-at-rest design.
- The entity model in section 6: InboxItem, Event, EventOverride, Presence, Task, Project, Thread, Anchor (declared & pulled only), DietaryProfile, Meal, Recipe (optional), GroceryItem, PantryItem, UseFirstItem, CompletionLog.
- Calendar aggregation from external ICS sources (read-only); hub-native calendar (read-write) for family events.
- Free-text meal planning with optional recipe linkage; meal-to-grocery pipeline.
- Chores as recurring Tasks with completion log; no fairness algorithm.
- Per-item privacy with no admin reveal; two-tier governance (admin / member).
- Three-level notification model (Critical / Today / Soft); no automatic escalation.
- Voice intents — the ten in section 12.
- EN and NL languages, per-member preference.
- Local-first storage on the hub; manual data export.
- Email forwarding address for capturing externals (manual; not parsed automatically).
- Weather display.

**Post-MVP (v1 and beyond):**

- Home Assistant integration (inbound events only).
- On-device geofences in the mobile app (strictly client-side).
- Photo-based capture (push-to-shoot, single-shot, no facial detection).
- Cloud sync with end-to-end encryption.
- Anchor surfacing — gentle, infrequent, in empty space.
- Recipe promotion suggestion.
- Self-calibration nudges for notification miscategorisation.

**Explicitly not in MVP, possibly never:**

- Auto-assignment or fairness scoring for chores.
- Gamification of any kind — streaks, badges, points, confetti.
- Email inbox parsing or school-portal scraping.
- OCR-based homework capture.
- Calorie/nutrition tracking; expiry-date alerting.
- Routing, commute estimates, traffic-aware reminders.
- Direct grocery delivery API integration.
- Always-listening voice; facial recognition; biometric capture.

## 6. Data model

### 6.1 Premise

The data model is informed by the principles above. It distinguishes between a *conceptual* layer (open loops, the unified inbox, the surfacing stream) and the *structural* layer (typed entities with appropriate fields, queries, and constraints).

The temptation to collapse everything into a single `Item` table with a `type` field is resisted. Type-specific entities give us appropriate fields, queryability, and clean affordances per type. The unified experience is provided at the inbox and surfacing layers, not by flattening the schema.

### 6.2 The Open Loop concept

An "open loop" is a conceptual property shared by several entity types: a grocery item, a Task, an Event, a Thread, a Project, a Meal, an Anchor — all are things the mind would otherwise hold. The system absorbs that holding.

This is delivered by two cross-cutting layers above the entities:

- **Capture (the inbox).** One unified intake, free-form, type-optional.
- **Surfacing (the stream).** One query, returning a ranked mixed-type list for the Today view.

### 6.3 Inbox

```
InboxItem {
  id, rawText, capturedBy, capturedAt,
  source(voice|touch|mobile|share|forward-email),
  attachments[],
  triagedTo,                                # id of the typed entity it became
  triageState(untriaged | typed | dismissed | kept_as_note)
}
```

Triage is optional and deferrable. The system does not pester for categorisation. Items may remain `untriaged` indefinitely; the inbox is itself surfacable when the user wants to skim un-triaged items.

### 6.4 Surfacing

The Today and Week views pull from a single ranked query across Event, Task, Project milestones, and Thread prompts. Anchors do not appear automatically. Ranking inputs include time proximity, stated priority, per-person filtering, quiet hours, and Presence.

The empty state is real: when nothing rises above the threshold, the view says so calmly. Tone is a property of this layer, not of items.

### 6.5 Entities

#### Cross-cutting fields

Every entity carries: `id`, `createdBy`, `createdAt`, `updatedAt`, `visibility(family | adults | private)` with per-member overrides, `ownerId?`, `tags[]`. Free-form tags do the work that fixed taxonomies traditionally try to do.

#### Event *(MVP)*

```
Event {
  title, startAt, endAt, allDay, timezone,
  location?, attendees[], reminders[],
  source { kind: native|google|apple|ics,
           externalId?, calendarId?,
           readOnly: bool, lastSyncedAt? }
}
```

The `source` field is essential. The UI uses it to decide what edit affordances to offer — full controls for native events; create-override-only for read-only externals.

#### EventOverride *(MVP)*

Local overlay applied to instances of read-only external events. Lets the household record "bin day moved because of King's Day" without writing back to the source.

```
EventOverride {
  sourceEventId, instanceDate,
  action(cancel | reschedule | annotate),
  payload, createdBy, createdAt
}
```

#### Presence *(MVP)*

Represents a member's or guest's availability over a time window. Not a calendar event — a separate concept that *affects* scheduling and rotation decisions.

```
Presence {
  memberId?, guestProfileId?,                # one or the other
  state(home | away | offshore | with_other_parent | traveling | unavailable | visiting),
  from, until, note?,
  affectsChoreRotation: bool
}
```

Read by chore logic, meal-warning logic, Today surfacing, and Kid Mode. Shared-custody alternation is expressed as recurring Presence windows.

#### Task *(MVP)*

```
Task {
  title, notes?, ownerId?, assigneeIds[],
  eligibleMemberIds?,                        # who may take this on
  currentAssigneeId?,                        # default; freely changeable
  dueBy?, earliestAt?,                       # window, not point-in-time
  effort?, priority?,
  status(open | doing | done | skipped),
  recurrence?,
  requiresAck?(parent | none),               # owner-only, opt-in
  projectId?,                                # soft membership
  checklist[], attachments[], tags[]
}
```

No `difficulty`. No `fairness_score`. Fairness is a computed view shown to humans, never stored.

#### Project *(MVP)*

Multi-step, long-horizon. Optional parent container; tasks reference it via soft `projectId`.

```
Project {
  title, description?,
  status(active | paused | done | archived),
  horizon?,
  milestones[ { title, targetDate?, doneAt? } ],
  attachments[], location?, ownerId?
}
```

Archiving a Project does not delete tasks that referenced it. The Project is a *view*, not a container with strict ownership.

#### Thread *(MVP)*

Relational, no completion state. Designed for social maintenance — friends, extended family, anyone whose connection wants occasional attention.

```
Thread {
  title, ownerId?, desiredCadence?,         # soft
  lastTouchedAt?, channelHint?, notes?
}
```

No "done" — only "touched base". No streaks, no overdue red. The UI affordance updates `lastTouchedAt`.

#### Anchor *(MVP — declared and pulled only)*

A standing claim on time and attention that is explicitly *not* an obligation. "Guitar". "Long walks alone". "Reading fiction in the bath".

```
Anchor {
  id, ownerId, title,
  preferredWindows?,                        # soft
  notes?, createdAt
}
```

MVP scope: declare, edit, delete; pull-only view ("what about me?"). No automatic surfacing. No tracking of compliance. Auto-surfacing is post-MVP and only when the obligation layer is mature enough to reliably distinguish "you are caught up" from "you haven't entered everything yet".

#### CompletionLog *(MVP)*

```
CompletionLog {
  id, taskId, instanceDate,
  completedBy, completedAt,
  notes?, skipped?: bool
}
```

The visible record of who did what. The substrate for the household's own fairness conversations. The system does not derive a score from it; it just displays the facts.

#### DietaryProfile *(MVP)*

Attached to a name. Not a CRM, not a contact entity.

```
DietaryProfile {
  id, name, relationship,
  safetyFlags[],                            # gluten, peanuts, etc.
  preferenceFlags[],                        # vegetarian, no pork, etc.
  prepNotes,                                # cross-contamination, prep care
  linkedMemberId?
}
```

Safety flags produce acknowledgement-required warnings when conflicting meals are planned. Preference flags produce passive notices. Neither blocks. Both respect Presence.

#### Meal *(MVP)*

```
Meal {
  id, date, slot(dinner | breakfast | lunch | other),
  label,                                    # free-text; default form
  recipeId?,                                # optional link
  notes?, servings?, guestCount?
}
```

Most meals are labels, not recipes. Linkage to a Recipe is opt-in and enables auto-grocery generation.

#### Recipe *(MVP, optional)*

```
Recipe {
  id, title, defaultServings,
  ingredients[ { name, qty?, unit?, optional? } ],
  steps[],                                  # optional
  tags[], allergens[], notes?
}
```

#### GroceryItem *(MVP)*

```
GroceryItem {
  id, name, qty?, unit?, category?,
  preferredStore?,
  source(manual | from_meal | from_pantry | from_recipe),
  sourceRefId?, checked: bool, notes?
}
```

Grouping by category by default; by `preferredStore` optionally. No aisle-order sorting in MVP. No automatic deduplication; user resolves duplicates in the moment.

#### PantryItem *(MVP)*

```
PantryItem {
  id, name, unit?,
  currentLevel(empty | low | ok | full),    # coarse enum, not quantity
  lowThreshold(low | empty),
  preferredStore?,
  dateAdded?, lastUpdatedAt
}
```

Levels update as side effects of normal activity (chore checklists, in-meal prompts, manual update). Auto-list when level ≤ threshold.

#### UseFirstItem *(MVP)*

```
UseFirstItem {
  id, name, addedAt, addedBy, notes?
}
```

A note-to-self surface. No expiry date, no urgency level, no notifications.

### 6.6 Time, recurrence, and cadence

**Time zones.** Events store their time zone. Native hub events default to Europe/Amsterdam. The hub always displays home time. The mobile app defaults to home time, with optional secondary local-time display for events the traveler is personally attending. Place-anchored events (school, bins) show home time only.

**Recurrence policy.** For externally-sourced recurring events, trust the source for holiday handling. For native recurring Tasks/Events: items tagged `chore` or `household` skip NL public holidays; everything else runs regardless. All native recurring items anchor to local wall-clock time (DST transitions don't move them). Deleting one instance of a recurrence creates an EventOverride for that instance only; "delete the whole series" requires an explicit confirmation.

**Recurrence DSL (draft).** Human-readable strings parsed into rules:

```
every=week; on=Mon
every=month; on=1st Sat
every=2 weeks; on=Thu
every=year; on=October
```

**Cadence (softer).** Used by Thread `desiredCadence` and post-MVP Anchor surfacing:

```
~2 weeks
~3 months
~yearly
```

The tilde is semantic: cadence is a preference, not a rule. Lapses do not trigger alarms.

## 7. Calendars and time semantics

Amity is a calendar **aggregator**, not a calendar source of truth. External calendars own their events; the hub displays them; writes go to a small hub-native calendar for family-coordination events that don't belong to any external source.

**External sources in MVP** (all read-only): school ICS feeds per child, sports/club ICS feeds, municipal afvalkalender (Zutphen: plastic, paper, GFT, rest), public NL holiday calendar, adult members' personal Google/Apple calendars.

**Hub-native calendar:** the only thing the hub writes to. Family events with no obvious external home live here.

The aggregator architecture matches the project's humility: Amity does not try to replace Google Calendar or be the system of record for a child's school. It adds value by aggregating, filtering through Presence, and presenting calmly.

## 8. Chores and home maintenance

### 8.1 Posture

The system records chores, displays the schedule, logs completions, and stays out of the way. It does **not**: auto-assign via any algorithm, compute a fairness score, suggest reassignment based on perceived imbalance, or apply emoji/colour/escalating tone to overdue items.

It does: show whose name is currently on each chore (a default, freely changeable in one tap); keep a visible CompletionLog; surface the work that needs doing.

Fairness is something humans do to each other. The system makes the facts visible. The humans negotiate.

### 8.2 Model

Chores are Tasks with `eligibleMemberIds`, `currentAssigneeId` (a default, not an assignment), tags, and recurrence. Reassignment is one tap, no reason required. Skipping writes a CompletionLog entry with `skipped=true`; no roll-forward, no guilt-debt accumulation.

### 8.3 Pre-seeded tags

`chore`, `errand`, `maintenance`, `outdoor`, `kid`, `garden`, `seasonal`. Free-form additions allowed. Tags drive default behaviour (holiday-skip for `chore`, weather hint for `outdoor`, kid-mode visibility for `kid`) but never enforce taxonomy.

### 8.4 NL seasonal reference

Indicative, not exhaustive. Every household tunes its own.

| Item                          | Cadence              | Modelled as                                      |
| ----------------------------- | -------------------- | ------------------------------------------------ |
| Chimney sweep                 | yearly, October      | Recurring Task, tag `maintenance`                |
| Gutters cleaned               | yearly, November     | Recurring Task, tags `maintenance` `outdoor`     |
| Boiler service                | yearly (contract)    | Recurring Task: "verify done & file invoice"     |
| Bike spring tune-up           | yearly, March        | Recurring Task                                   |
| Bike autumn lights/tyres      | yearly, October      | Recurring Task                                   |
| Heating-on / heating-off      | twice yearly         | Two recurring Tasks                              |
| Storm windows / draft check   | yearly, October      | Recurring Task, tag `outdoor`                    |
| House repaint                 | every 7 years        | Project, with child tasks                        |
| Garden — yearly plan          | seasonal             | Project + soft-membership recurring tasks        |
| Garden — recurring care       | bi-weekly in season  | Recurring Tasks, optional `projectId`            |
| Sinterklaas preparation       | yearly, November     | Project (gifts, schedule, food)                  |
| Vacation preparation          | per trip             | Project                                          |

### 8.5 Supplies coupling

Chores do not directly add items to grocery lists. The coupling is *pantry-driven*: items have low-threshold pantry records, and either explicit checks (a chore checklist step asking about supply level) or manual updates lower the pantry value. When level ≤ threshold, the item lands on the grocery list.

## 9. Meals, groceries, pantry

### 9.1 Posture

The meal-to-groceries pipeline is one of the few places in Amity where real automation pays for itself. The posture is **structure grows with use** — meals are labels by default; recipes are optional structure invested in when their auto-grocery payoff is wanted.

Amity is not a dietician, nutrition tracker, calorie counter, substitution engine, or inventory system. It is a planner for *what we're eating, who's eating it, what we need to buy*.

### 9.2 Meals: dinner only, weekly

Breakfast and lunch are pattern-based and not tracked. Exceptions (a school-trip lunchbox, a guest brunch) are ad-hoc Meal entries.

A "today" slot for unplanned cooking lets the user retroactively update what was actually eaten — the history stays honest.

### 9.3 Dietary needs

Two-tier model. **Safety flags** (allergies, medical exclusions) produce acknowledgement-required warnings when conflicting meals are planned. **Preference flags** (vegetarian, religious observance, dietary patterns) produce passive notices. Neither is a hard block — the user may have context the system doesn't.

Both apply to household members *and* recurring guests via DietaryProfile. When a guest is visiting (Presence: `visiting` with `guestProfileId`), their flags become active for that window. Prep notes (cross-contamination, special handling) surface alongside warnings.

### 9.4 Grocery list

Three sources: manual capture, from a planned meal (via recipe ingredients or a per-meal "what do you need?" prompt), from the pantry (auto-listed when below threshold).

No automatic deduplication. No substitution rules. No aisle-order sorting. Groups by category (default) or by preferred store (optional).

### 9.5 Pantry: coarse levels

`currentLevel` is an enum (empty / low / ok / full), not a number. People don't count cups of flour. Numeric tracking is exposed via `unit?` for genuine special cases. Levels update as side effects of normal activity, not as a separate inventory chore.

### 9.6 Use-first list

No expiry tracking. A simple free-text list of items the cook wants to use this week. Populated manually, viewed in the meal-planning context as a hint. Done items are marked done and disappear.

## 10. Children, homework, Kid Mode

### 10.1 Posture

Children are full users of Amity to the extent their age allows, not objects managed through it. Where parental interest and child autonomy diverge, the design defaults to protecting the child's emerging autonomy.

**Amity does not do** for children: automated parent reports, gamification (no streaks, badges, confetti), reading logs, screen-free reminders, "smart" suggestions about how a child should spend their time, OCR-based homework capture.

**Amity does**: hold a child's commitments in the same model as everyone else's; show a calm, age-appropriate "Today" per child; provide kid-invoked focus timers for homework; honour per-item privacy; stay out of the parent–child relationship except where the household has explicitly invited it.

### 10.2 Homework

There is no dedicated Homework entity. Homework is a Task with conventional tags (`homework`, optional `<subject>`). Capture is manual (typed or voice); photos attach as references, never as the input mechanism. OCR is rejected for MVP — too unreliable to be trusted with a child's grade.

### 10.3 Parent involvement

**Focus timers** (15/20/25 minutes): kid-invoked. Outcome not logged anywhere parents see.

**Approvals** (`requiresAck: 'parent' | 'none'`): per-item, set by the **owner only**. Default none. The parent cannot retroactively add this flag.

**Explicitly not in this cluster:** automated reading logs, device-free time prompts, screen time tracking, performance scoring.

### 10.4 Privacy

Per-item visibility, set by the owner. **No admin reveal** — no parental override exposes a child's private item. Building one creates a coercive-control vector.

**Age threshold for private status.** Household-configurable (default 13). Children below cannot mark items private *from* parents (parents may still mark items "child only" — a different psychological act).

**Calendar density remains visible across privacy boundaries.** A teen marking 16:00–18:00 Tuesday private shows as busy in that window, without exposing what fills it. This answers the legitimate concern about family obligations without violating the privacy guarantee.

### 10.5 Kid Mode

A view, not a separate app. Each child sees a personalised "Today" drawn from the same data, presented appropriately: short, plain, large tappable items; plain language; no emojis pushed by the system. NFC tap or PIN/emoji to switch context. Session timeout returns to shared Today view.

No streaks, no badges, no points, no confetti, no levels, no system-issued evaluation. The reward for a child doing their homework is the parent's quiet acknowledgement, the avoided stress, eventually the grade. Amity stays out of that reward loop.

## 11. Notifications and attention

### 11.1 Posture

Every notification fires at a cost: it interrupts, it teaches the household whether to trust the channel, and it shapes attention. The default volume is near-silent. When in doubt, the system does not speak.

### 11.2 Three levels

**Critical.** Real-world consequence if missed, recoverable only by acting now. Push to relevant members' phones; single audible chime on hub; pulsing amber LED. Pierces quiet hours. Target frequency: ~3 per week.

**Today.** Things that matter today, that the family implicitly knew about. Visible in hub Today view. No push to phones (per-item opt-in exists). No chime. Silent during quiet hours; appears in hub the next morning.

**Soft.** Everything else. Visible only when the user opens the relevant view. No alert, no chime, no LED change. Present, not announced.

### 11.3 Quiet hours and Presence

Default 22:00–07:00 per member, configurable. Critical pierces; Today and Soft simply don't send during these windows. No morning digest — the Today view *is* the digest.

Members in `away`, `offshore`, or `with_other_parent` Presence states receive only items that name them directly. They don't get general household nudges (bins, family dinner) because they're not there.

### 11.4 Repeat, not escalate

A Critical that isn't acknowledged after 4 hours repeats — same recipient, same channel, single repeat. After that, no further automatic action.

**No automatic escalation to a second person.** Escalation chains imply the system has a stake in compliance; it doesn't. Manual reassignment in one tap exists; the human escalates, not the system.

**Per-item opt-in escalation** exists for genuine cases (medication for a dependent member): `escalatesTo: [memberId]`. Rare, deliberately set.

### 11.5 Hub at rest

Permanent fixtures: day and time (large), one-line weather, status patch ("3 things today" or "nothing today" — tapping opens Today view), ambient LED.

That's the whole baseline. The hub at rest looks more like a calm clock than a dashboard.

### 11.6 Ambient LED

Off / very dim white: nothing pending. Soft warm white: Today items present, no urgency. Pulsing amber: Critical alert active, unacknowledged. No red, ever. No green "all good" — absence is its own message.

### 11.7 Self-calibration (post-MVP)

If Critical alerts fire more than ~5/week consistently, the hub surfaces a quiet note inviting categorisation review. Once dismissed, doesn't return for a month.

## 12. Voice and hub UX

### 12.1 Push-to-talk only

The microphone activates only on explicit user invocation — a button on the hub (held or tapped) or in the mobile app. While listening, visual feedback shows the state. When the user finishes speaking, the microphone disengages immediately. No buffering, no ambient capture.

On-device transcription. Audio clips are processed and discarded; only transcribed text persists, briefly, while the intent is being parsed.

Cloud-fallback for higher accuracy is opt-in per device, never default, with full disclosure of what the trade-off is.

### 12.2 The top 10 voice intents

| # | Intent | Example | Action |
|---|--------|---------|--------|
| 1 | Add to groceries | "Add milk to groceries." | Creates GroceryItem. |
| 2 | Read meal plan | "What's for dinner?" / "What's for dinner Friday?" | Reads Meal label. |
| 3 | Add to use-first | "Add asparagus to use-first." | Creates UseFirstItem. |
| 4 | Add task | "Add task wash bikes Saturday." | Creates Task. |
| 5 | Note | "Note: ask Anneke about the holiday." | InboxItem, untriaged. |
| 6 | Look up event | "When is the school trip?" | Searches Events. |
| 7 | Who's on the chore | "Who is doing the bins this week?" | Reads currentAssigneeId. |
| 8 | Mark done | "Mark dishwasher done." | CompletionLog entry. |
| 9 | Snooze | "Snooze the dentist to tomorrow." | Defers a Today item. |
| 10 | Cancel | "Cancel." | Aborts current interaction. |

Five are capture, not query. The Note intent is the safety net — when in doubt, dump it; the inbox catches it.

### 12.3 Error recovery

- **No silent failures.** Every utterance produces an action, a clarifying question, or an inbox capture.
- **One follow-up, then fallback.** If still ambiguous after one round, the transcribed text becomes an InboxItem ("I've added that as a note for you to check").
- **Confirmation on success.** Brief audio + visual confirmation. "Milk added to groceries."

### 12.4 Accessibility

- **Typography:** Atkinson Hyperlegible. Single typeface across the system. No "fun" fonts, including in Kid Mode.
- **Type sizing:** legible at ~1m (conversational distance from across the kitchen), not arm's length.
- **Touch targets:** minimum 60×60 px on the hub; 80×80+ for primary actions.
- **Colour:** never the only carrier of information. Position, text labels, and shape carry; colour augments. WCAG AA minimum, AAA where feasible.
- **Languages:** EN and NL at launch. Per-member preference. Dutch date convention (day-first, 24-hour) by default.
- **NL voice quality caveat:** on-device transcription for Dutch is currently less reliable than for English; MVP accepts this. Inbox-fallback ensures degraded recognition produces useful results, not failures.

### 12.5 Hub auth

NFC tap (fastest), PIN, emoji code (kids — more memorable at age 7 than digits). No facial recognition, no biometrics, no presence-based auto-switch. Session timeout 5 minutes idle, returning to shared Today view (household-visible items only).

## 13. Integrations

Every integration is evaluated against the boundary checklist in section 2. Most fail; the survivors are listed here.

### 13.1 MVP integrations

- **Calendar ICS sources** (read-only): school per child, sports/club, municipal afvalkalender, NL public holidays, personal Google/Apple calendars (read-only on hub).
- **Weather:** city-level location configured explicitly; weather queried from a public service.
- **Email forwarding address:** per-household inbound address; forwarded emails create InboxItems with content attached. The system does *not* parse inboxes directly.

### 13.2 Post-MVP

- **Home Assistant** (inbound events only): presence detection, appliance signals, local-network sensor data. Runs over local network; no HA cloud component.
- **On-device geofences:** strictly client-side. Phone knows its location; phone shows local list. Location never leaves the device.
- **Photo-based capture:** explicit single-shot, on-device processing where possible, no facial detection.
- **Cloud sync:** end-to-end encrypted, opt-in.

### 13.3 Rejected

- Email inbox parsing (privacy cost too high; replaced by forwarding pattern).
- School portal scraping (fragile, requires inbox access or per-portal credentials).
- Routing, commute estimates (require sending location to third parties).
- Direct grocery delivery API integration (creates commercial relationship; user-initiated export instead).
- Anything always-on, anything outbound to advertising-capable parties, anything that creates third-party leverage.

## 14. Privacy, governance, safety

### 14.1 Data and deletion

Data lives on the household's own hardware. Cloud sync is opt-in and end-to-end encrypted; the cloud party holds only ciphertext.

Member deletion removes the FamilyMember, owned tasks, private items, completion log entries, DietaryProfile, Anchor list, and Threads. Shared items with multiple assignees keep the records (anonymised with a tombstone). Full data export offered before deletion. Children's data: same path as adults, no special weaker route.

Audit log (member/visibility/integration changes) kept 90 days, admin-visible only.

### 14.2 Abuse prevention

**No monitoring features.** The system does not implement features whose purpose is one member tracking another.

**Admin power is content-asymmetric in only one direction.** Admins manage settings and structure; they have no extra access to other members' private content.

**Help-finding affordance.** A small menu item in every member's settings linking to local domestic-violence and family-support resources (NL, EN). Accessing it generates no audit trail entry.

### 14.3 Governance: two tiers

**Admin:** at least one adult member; multiple permitted. Powers: member management, household-level settings, hub config, full data export. Cannot read others' private items.

**Member:** everyone else. Manages own content and visibility. Participates in everything visible to them.

PIN/passphrase resets self-service via recovery method; admin-assisted only as fallback.

### 14.4 Change log

Factual record of changes (who reassigned what, who completed what, who modified shared content). Visible to all family members, not just admins. Retention 90 days. No "conflict resolution", no "blame view", no notifications when shared items change (that would be a monitoring feature in disguise).

### 14.5 Split and merge

Departing member: exports their own data plus jointly-owned data they choose to take; can spin up a new household instance. New member joining: standard add-member flow plus optional import. No special "support divorce" workflow — the architecture just doesn't trap data.

## 15. Hardware and the home node

### 15.1 The hub is a home node

The hub is the first form of a household-owned home node. The MVP runs on a tablet for accessibility; the architecture is designed to migrate to a small home server without rewrite.

### 15.2 Form factors

- **All-in-one touchscreen computer:** single device with compute, storage, and a kitchen-suitable display. Simpler install: one device, one power cable, one wall mount.
- **Separate compute + display:** small home server (Raspberry Pi 5 / mini-PC + NVMe + UPS) in a utility area; touch display in the kitchen as one client. Quieter, easier hardware refresh.
- **MVP shortcut:** Android tablet in kiosk mode. Lowest-friction entry point for a first family. Architecture still permits later migration.

### 15.3 Storage

1TB minimum on the home node. The planner's own data is tens of MB; storage capacity is for photo attachments, forwarded email contents, eventual voice transcript history if retained, and the user's own files held on their own infrastructure.

The planner is the first application; the home node is the platform.

### 15.4 Power and reliability

Always-on. Fanless or quiet thermals. Low power draw (10–30W). Small UPS or battery backup. PoE supported on the Pi route.

### 15.5 Display behaviour

Always-on with daylight dimming. Hub at-rest display (section 11) is also the screensaver — a calm clock with weather is what the device shows when nothing else needs attention.

## 16. Internationalization

EN and NL at launch. Per-member language preference (not per-household — mixed-language households are common).

Dutch date conventions (day-first, 24-hour) by default; per-member override.

NL public holiday calendar loaded at install, refreshed annually. Drives holiday-skip for chores.

NL regional school holiday schedules (Noord/Midden/Zuid) configured per child during onboarding.

Municipal afvalkalender per household via postcode + house number.

Values stored locale-neutral (ISO datetimes, units with explicit denominations); rendered at display time.

## 17. Long-term direction

> The hub is the first form of a household-owned home node. The MVP runs on a tablet for accessibility; the architecture is designed to migrate to a small home server (compute + storage + always-on) without rewrite. All-in-one touchscreen computers can serve as both server and primary display; the architecture also supports separate compute with a touch display as one client.
>
> Long-term, the home node becomes the substrate for additional household-owned services, with Amity as its first application. The project's ultimate trajectory is *post-cloud household computing*: data the user owns, on hardware the user controls, with no commercial party in the middle.
>
> Distributed peer-to-peer backup via networks like I2P or Tahoe-LAFS — encrypted, erasure-coded, hosted on the hardware of trusted peers — is one explicit candidate for that trajectory. The technology is mature; the social and bootstrap challenges are real. Not for MVP, but the architecture today should not foreclose it tomorrow.

This direction explains, to future contributors and to the project's author in years to come, why certain MVP decisions are made the way they are: the categorical privacy commitments, the rejection of cloud integrations, the home-node storage sizing, the insistence on local-first data. They are all early forms of the trajectory above.

## 18. Roadmap and validation

### 18.1 Quarter 1 sketch

Weeks 1–2: data model and core service in Rust; inbox + capture pipeline; a usable Today view on the tablet form factor.

Weeks 3–4: Meals + Lists + PantryItem; the meal-to-groceries pipeline end-to-end without recipes.

Weeks 5–6: Calendar aggregation (ICS feeds); EventOverride; recurrence engine.

Weeks 7–8: Tasks, recurrence, CompletionLog; chore views.

Weeks 9–10: Notifications (the three-level model); hub-at-rest; LED.

Weeks 11–12: Polish, accessibility audit, real-household pilot.

### 18.2 Validation before further scope

- **Paper prototypes** of the four views with a real family; observe for three evenings.
- **Cognitive walkthrough** with five scripted tasks (add milk, swap bins, plan dinner, log homework, add event).
- **Privacy drill:** try to access a teen's private item from admin; verify no path exists.
- **Data export rehearsal:** populate sample data; perform full export; verify GDPR basics.
- **Outage drill:** simulate no-internet weekend on the hub; verify graceful read-only or fully-functional offline mode.

### 18.3 Definition of done (MVP)

A family of 3–6 can run an entire week on Amity:

- Menu planned, groceries generated and checked off on mobile.
- Chores visible; assignments default and changeable; CompletionLog populated.
- Calendar aggregated; at least 3 new events created via hub or voice.
- All critical reminders fired on time.
- Data export validated.
- No fatal crash in a week.
- Offline mode graceful.

## 19. Open questions

- Cloud sync provider strategy: self-hosted by household, optional managed service, both? Strong preference for self-hosted; details to design.
- Mobile app: native (React Native? Tauri?) vs. PWA. Architectural separation already permits either; choice can defer to a later phase.
- Voice transcription model selection (Whisper variants, faster local alternatives) and NL quality benchmarking.
- Recurrence DSL: full-featured RRULE compatibility, or the smaller human-readable DSL sketched in section 6.6, or both as a translation layer?
- Hub OS choice for the home-node form factor (NixOS, Debian, custom appliance image).
- License selection and copyleft posture (see philosophy and licensing discussion separate from this brief).

---

*End of canonical brief.*
