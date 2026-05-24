## §21D — Calendars & Time Semantics (answered)

*These answers refine and extend the original brief. The underlying decisions are reflected in the updated §3 data model (Event, EventOverride, Presence) and in the time-zone / recurrence policy section.*

### 12. Authoritative calendars — which sources, and who can write back

The system takes an **aggregator-style** approach to calendars: external sources own their events; the hub displays them; writes go to a small native hub calendar for family-coordination events that don't belong to any external source.

Concretely:

- **Read-only external sources** (the hub never writes back to these):
  - Kids' school ICS feeds, per child, per school.
  - Sports club / activity ICS feeds where available.
  - Municipal afvalkalender (Zutphen): plastic, paper, organic ("GFT"), and rest. Likely four separate recurring series under one source.
  - Public NL holiday calendar (loaded once, refreshed annually).

- **Read-write personal sources** (treated as external; the user may choose to mirror events out, but the hub does not assume it can):
  - Adult members' personal Google or Apple calendars (work, social).
  - These remain owned by the individual. The hub displays them in the family view but does not modify them.

- **Hub-native calendar** (the only thing the hub itself writes to):
  - Family events created on the hub or mobile app that have no obvious external home: "dinner with the Jansens Saturday", "Anneke's birthday party", "kids' dental appointment together".
  - Native events are first-class in the model: they are not a fallback when external syncs fail; they are deliberately the home for *family-coordination* events.

Rationale: external services already have authoritative calendars for the domains they own (schools, municipalities). The hub adds value by aggregating them, not by trying to replace them. The small native calendar exists so that family events have a stable home that doesn't depend on which adult happens to be the "calendar owner" in Google.

### 13. Time zones and travel — home time vs. local time

The system uses **place anchoring** for events, not viewer anchoring.

- The hub is a physical device in the kitchen in Zutphen. It always displays Europe/Amsterdam time. There is no concept of "viewer time zone" on the hub itself.
- The mobile app defaults to home time. When a member is traveling, the app may show a secondary local-time line for events they are personally attending; place-anchored events (school, bins, kid's club) show home time only, because the traveler isn't attending them and the events haven't moved.
- Each event stores its time zone (default Europe/Amsterdam for native; whatever the source declares for external). This is robust to future cases like an adult relocating temporarily.

Travel and presence are not modelled as calendar events. They are modelled as a separate `Presence` entity (see §3.5), which the scheduling and rotation logic consults independently. "Wife offshore for 6 weeks" is a Presence window, not an event — and it correctly removes her from chore rotation, dinner planning, and bin scheduling for that period without any per-feature special-casing.

For shared custody, children's alternating-week schedules are expressed as recurring Presence windows (state alternates between `home` and `with_other_parent`). The rest of the system reads this and behaves accordingly.

### 14. Recurrence edges

Defaults, with per-item override possible:

- **Public holidays.** For externally-sourced recurring events, the source is trusted to handle holidays (Dutch municipalities and schools publish corrected ICS feeds). For *native* recurring Tasks and Events: items tagged `chore` or `household` skip on declared NL public holidays; everything else runs regardless. The NL holiday list is loaded from a maintained public source at install and refreshed annually.

- **Daylight savings.** All native recurring items anchor to **local wall-clock time**. "08:00 every Thursday" stays at 08:00 on the household's clock through DST transitions. UTC anchoring is not exposed to users.

- **5th-week and irregular anomalies.** The recurrence DSL supports unambiguous forms ("first Saturday of the month", "last Thursday of the month", "every other Thursday starting from [date]"). No special policy needed beyond the DSL itself.

- **Deleting one instance of a recurring item.** The default action is to cancel only that instance, recorded as an `EventOverride` with `action=cancel`. An undo affordance offers "actually, delete the whole series" within a short window after the action. The default is the safe direction.

These defaults are reflected in §3.7 of the data model. They can be overridden per item where needed, but the goal is that they almost never need to be.
