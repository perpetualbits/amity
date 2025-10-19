# Family Organizer – Product Brief & MVP Plan

_Last updated: 2025‑10‑19_

## 0) Vision
Help families share the cognitive load of home life. Replace the kitchen chalkboard + paper calendars with a joyful, shared system that:
- Captures everything (events, chores, shopping, meals, maintenance, kids’ homework, clubs).
- Assigns responsibility fairly (breaks gendered defaults; supports rotation & load‑balancing).
- Surfaces the right thing at the right time (context‑aware nudges, glanceable UI).
- Works for everyone (kid‑friendly, non‑techy‑friendly, multilingual, neurodiversity‑aware).

Primary interface: a wall‑mounted touchscreen “hub” in the kitchen. Secondary: iOS/Android apps. Optional: voice listener in the kitchen.

---

## 1) Personas & Core Jobs-to-be-Done
**Care Coordinator (often mom today, but we aim to distribute):** wants visibility, delegation, and fewer mental checklists.
**Project/House Lead (often dad today):** wants planning views for projects, budgets, maintenance cycles.
**Teens/Kids:** want clear, bite‑sized tasks, streaks, and autonomy; privacy for personal items.
**All Adults:** want calendars unified, no duplicate entry, simple recurring workflows.

**JTBD Examples**
- “When the week starts, I need a menu plan and auto‑generated grocery list.”
- “When the bins are due, I want the right person reminded the night before.”
- “When a kid’s club fee is due, I want it surfaced alongside monthly budget.”
- “When homework is assigned, I want tiny check‑ins and parent sign‑off if needed.”

---

## 2) Scope for MVP (first 90 days)
**Must‑have**
1. **Kitchen Hub UI** with 4 tabs: _Today_, _Week_, _Meals_, _Lists_.
2. **Shared Task model** with owners, due, recurrence, difficulty, effort estimate, and rotation rules.
3. **Family Calendar** (read from Google/Apple/ICS) + write new family events.
4. **Meal Planner** (weekly board) → **Grocery List** by recipe/ingredient rules.
5. **Chores** with rotation (per day/week) + fairness/load‑balancing.
6. **Notifications**: push to phones; on‑hub chimes; optional LED light cue.
7. **Voice intents** (limited): “Add milk”, “What’s for dinner?”, “Who takes out bins?”
8. **Offline‑first data** (local hub) with cloud sync; read‑only mode without internet.

**Nice‑to‑have (stretch)**
- Simple homework tracking per child (subject, due, checklist, parent sign‑off).
- Maintenance planner (e.g., filters, gutters, boiler, bikes) with seasonal cadence.
- Budget hooks (tags per task/recipe; export to CSV).

---

## 3) Core Concepts & Data Model (draft)
**Entities**
- `FamilyMember { id, name, role, avatar, age, preferences, devices[] }`
- `Task { id, title, notes, ownerId?, assigneeIds[], dueAt, startAt?, duration?, priority, tags[], location?, effort(1‑5), difficulty(1‑5), status(enum), checklist[], recurrence(rule), rotation(rule), createdBy, createdAt, attachments[] }`
- `Event { id, title, startAt, endAt, allDay, attendees[], source(cal|local), location?, reminders[], visibility }`
- `Meal { id, day, slot(enum: breakfast/lunch/dinner), recipeId?, notes, servings }`
- `Recipe { id, title, ingredients[{name, qty, unit}], steps[], tags[], allergens[], cookTime, prepTime }`
- `List { id, type(enum: groceries/todo/packing), items[{id, text, qty?, unit?, checked, category}], autopopulateRules[] }`
- `RotationRule { id, type(enum: round‑robin/weighted), members[], cadence(enum), constraints (e.g., age >= 10), exceptions }`
- `MaintenanceItem { id, title, cadence(rule), lastDoneAt, responsibleRule, materials[], costEstimate }`
- `Homework { id, childId, subject, title, dueAt, subtasks[], requiresParentCheck(bool) }`

**Recurrence/Rotation Rule DSL (human‑readable)**
- Recurrence: `every=week; on=Mon`; `every=month; on=1st Sat`; `every=2 weeks; on=Thu`
- Rotation: `round_robin: [memberIds]; cadence=weekly; skipIfAway(true)`

**Fairness metric (alpha)**
- For each member: maintain rolling 28‑day _Effort Score_ = Σ(task.effort * completed) – Σ(debt)
- UI shows “Load Balance” bar + suggestions to reassign next rotation.

---

## 4) Interaction Model – Kitchen Hub
**Today Tab** (glanceable)
- Top row: current time, weather, next hard stop.
- “Now/Next” lane: current task/event cards per member (swipeable).
- Quick actions: +Task, +Event, +List item, +Meal note.
- Voice chip: “Say ‘Add milk’ or ‘What’s for dinner?’”.

**Week Tab**
- 7‑day columns with family rows (like your paper calendar). Drag‑drop tasks/events.
- Chore lane with rotation suggestions and fairness indicators.

**Meals Tab**
- Chalkboard‑style week menu; tap a slot to choose recipe; pantry check; generate grocery list.

**Lists Tab**
- Groceries grouped by store aisle; personal & shared lists; smart merge (no duplicates).

**Kid Mode**
- Large tiles; single “Today” list; streaks + small confetti moments; no ads, no distractions.

---

## 5) Mobile App (companion)
- Home: Today + notifications; quick capture (voice/text/camera for receipts/notes).
- Per‑member filter: “Show only my stuff” toggle.
- Offline capture: queue and auto‑sync.

---

## 6) Voice Intents (MVP)
- “Add [item] to groceries.” → List item.
- “What’s for dinner [day]?” → Meal card.
- “Who takes out the bins [this week]?” → Rotation answer.
- “Add event [title] on [day/time].” → Calendar entry.
- Wake phrase configurable; on‑device ASR preferred for privacy (Hotword + NLU).

---

## 7) Chore Rotation & Fairness (details)
- Configure eligible members, cadence, and exceptions (travel, exams, illness).
- Auto‑assignment proposes next assignee; one‑tap swap; logs reason.
- Fairness meter: shows rolling effort and “give a break” suggestion if someone’s overloaded.

**Edge cases**
- Missed chores roll forward with lower priority; avoid cascading guilt. Offer “skip w/ note.”

---

## 8) Meal → Groceries Pipeline
- Choose recipes per slot → ingredients merged → pantry subtract → store categories.
- Smart rules: “Always buy milk on Sunday”, “Double if guests ≥ 6”.
- Allergy/sensitivity flags per member → warnings on recipe selection.

---

## 9) Privacy, Safety & Roles
- Roles: Owner, Adult, Teen, Child, Guest.
- Visibility per item: Family / Adults / Private.
- Local‑first datastore on hub; E2E encryption for cloud sync.
- Voice redaction: wake‑phrase buffer only; no ambient recording.
- Data export/import (JSON/ICS/CSV). No lock‑in.

---

## 10) Integrations (phased)
- **Phase 1:** Google/Apple/ICS calendar read‑write; Reminders/Tasks import.
- **Phase 2:** Home Assistant (trash pickup schedule, presence, appliances), iCal garbage collection feeds.
- **Phase 3:** School portals (via email parsing / ICS); grocery delivery lists (export formats).

---

## 11) Hardware & Install
- **Hub options:**
  - Wall‑mounted Android tablet (budget) with kiosk app.
  - Raspberry Pi + 15.6\" touch display + 3D‑printed frame; PoE if possible.
- Always‑on with daylight dimming. Magnetic NFC tags for quick logins (kids tap badge).

---

## 12) Tech Stack (proposal)
- **Frontend:** React Native (mobile) + React (hub kiosk). Shared component library.
- **State:** Local IndexedDB/SQLite via RxDB or WatermelonDB; CRDTs for conflict‑free sync.
- **Backend:** Self‑hosted Go/Node service; WebSocket sync; optional cloud relay; Postgres.
- **Voice:** Local hotword (porcupine/snowboy alt), on‑device ASR where possible; fallback cloud.
- **Auth:** Passkeys + device‑scoped tokens; per‑member PIN/emoji lock for kid mode.

---

## 13) Success Metrics
- Weekly active family members ≥ 3.
- Reduction in “missed” critical tasks (bins, appointments) by ≥ 80% after 4 weeks.
- Close‑rate of assigned chores ≥ 85% with positive feedback.
- Self‑reported cognitive load reduction (survey) ≥ 30%.

---

## 14) Risks & Mitigations
- **Adoption friction:** Start with hub + minimal tabs; make capture 1‑tap/1‑sentence.
- **Privacy concerns:** Local‑first, minimal telemetry, clear export, no ads.
- **Over‑complexity:** Guardrails in UI; progressive disclosure; defaults tuned for families.

---

## 15) Roadmap (Quarter 1)
**Weeks 1–2**: clickable prototype for Hub (Today/Week/Meals/Lists). Usability test in family.
**Weeks 3–4**: implement Tasks, Meals, Lists; local storage; grocery merge rules.
**Weeks 5–6**: Calendar sync (Google/ICS); basic notifications; PoC voice intents.
**Weeks 7–8**: Chore rotation engine + fairness metric; Kid Mode v1.
**Weeks 9–10**: Offline‑first sync; conflict tests; data export.
**Weeks 11–12**: Polish, performance, accessibility; pilot in real‑world use.

---

## 16) Sample UX Copy & Screens
**Kitchen Hub / Meals (chalkboard style)**
```
Mon  Pasta pesto   |  Tue  Veggie chili  |  Wed  Stir-fry
Thu  Soup + bread  |  Fri  Pizza night   |  Sat  BBQ
Sun  Leftovers + salad
```
Buttons: [Generate Groceries] [Swap Meal] [Double Portions]

**Chore Rotation Card**
- _Bins this week:_ **Alex**  → [Swap] [Skip] [Done]
- _Dishwasher today:_ **Sam** → [Remind]

**Kid Mode “Today”**
- Big card: “Homework: Math worksheet (due Wed)” → [Start Timer 20m] [Done]
- Streak footer: 🔥 4‑day streak! High‑five!

---

## 17) Algorithms (brief)
**Fairness**: rolling window effort, with decay λ=0.9/week; suggest next assignee = argmin(expected_effort + fairness_penalty).
**Smart reminders**: bin night → remind 19:00 previous day; homework → micro‑nudge if 48h left & free window found.
**Pantry subtract**: maintain `PantryItem{name, qty, unit, lowThreshold}`; when item < threshold, auto‑add to list.

---

## 18) Open Questions (to iterate)
- Which calendars to sync first (Google vs Apple)?
- Do we need budgeting in MVP or just tags/CSV export?
- Do we support school homework portals initially, or manual entry only?
- What languages to support first (EN/NL)?

---

## 19) Definition of Done (MVP)
- A family of 3–6 can run an entire week from the Hub:
  - Menu planned, groceries generated and checked off on mobile.
  - Chores rotated fairly; notifications sent; 85% completion.
  - Calendar synced; at least 3 new events created via Hub/Voice.
  - All critical reminders fired on time (bins, meds, school forms).
- Data export validated; no fatal crash in a week; offline mode graceful.

---

## 20) Next Steps
1. Approve MVP scope above.
2. Choose hub hardware (tablet vs Pi) and mounting.
3. I’ll draft wireframes for the 4 tabs and voice micro‑flows.
4. Start a Git repo with schema stubs and a JSON export format.

---

## 21) Deep-Dive Discovery Questionnaire (answer succinctly; use bullets; mark unknowns)
### A. Mission, Scope, and Boundaries
1. **Primary mission**: If the system does only *three* things extraordinarily well, what are they?
2. **Non-goals**: What problems will we explicitly *not* solve in v1/v2 (e.g., budgeting, school portal scraping)? Why?
3. **Failure posture**: In what scenarios is it acceptable for the system to *do nothing* rather than risk wrong output?

### B. Family Model & Consent
4. **Household graph**: Who counts as a member (live-in, co-parent across homes, grandparents, au pairs, roommates)?
5. **Consent model**: What can adults see from teens? From children? Are there “sealed” items even owners can’t open?
6. **Shared custody**: How do we represent alternating‑weeks schedules and cross‑house data sharing?
7. **Guest access**: Temporary codes? NFC tap badges? Time‑boxed permissions?

### C. Fairness, Load & Motivation
8. **Fairness metric**: How do *you* define fair? (time spent, effort, unpleasantness, reliability, skill growth)
9. **Rotation rules**: What constraints exist (age ≥ X, allergies, strength, school nights, religious days)?
10. **Negotiation**: How do members propose swaps/deferments without shaming? (reasons, limits, cooldowns)
11. **Motivation style**: Points/badges/streaks vs. intrinsic celebrations? Family rewards vs. individual?

### D. Calendars & Time Semantics
12. **Authoritative calendars**: Google, Apple, Outlook, school ICS, sports teams? Who can write back?
13. **Time zones & travel**: How do events shift when a parent travels? “Home time” vs “local time” views?
14. **Recurrence edges**: Bin day on holidays, daylight savings jumps, 5th‑week anomalies—what are your expectations?

### E. Chores & Home Maintenance
15. **Chore taxonomy**: Daily, weekly, monthly, seasonal; indoor/outdoor; maintenance vs. tidying.
16. **Seasonality**: Garden tasks (Netherlands climate), gutters, boiler, bikes—what cadences?
17. **Supplies coupling**: Which chores imply automatic list items (bin bags, dishwasher salt, chicken feed)?

### F. Meals → Groceries → Pantry
18. **Meal planning style**: fixed weekly plan vs. rolling two‑week? Breakfast/lunch tracked or just dinner?
19. **Dietary rules**: Allergies, vegetarian days, halal/kosher, low‑carb, athlete macros—how strict?
20. **Grocery flows**: Multiple stores/markets? Substitutions? Minimum pantry levels? Bulk buying cadence?
21. **Waste minimization**: Leftover planning, “use‑first” list, expiry tracking—how proactive?

### G. Children & Homework
22. **Homework capture**: Manual entry vs. scrape email/portal vs. photo‑OCR of assignments?
23. **Parent involvement**: Approvals, timers (Pomodoro), reading logs, device‑free time prompts?
24. **Privacy**: Can a teen mark an item private? Can adults override? Under what policy?

### H. Notifications & Attention
25. **Notification budget**: Max N per day? Priority levels? Quiet hours per member? Home vs. away modes?
26. **Escalation**: If a critical item isn’t acknowledged, who gets pinged next and how (push, chime, light strip)?
27. **Glanceables**: What must be visible *at a glance* on the hub at all times?

### I. Voice & Kiosk UX
28. **Wake words**: Custom phrase? Physical mute? LED status? On‑device hotword required?
29. **Voice intents** (MVP list): Which 10 utterances matter most? (add item, who has bins, what’s for dinner…)
30. **Accessibility**: Large touch targets, dyslexia‑friendly fonts, color‑blind safe palette, Dutch/English support.

### J. Integrations & Automations
31. **Home Assistant**: Presence, trash schedule, washer/dryer done, doorbell events?
32. **Email parsing**: From which senders (school, clubs) should we extract dates/fees?
33. **Location signals**: Geofences (arrive near store → show list), commute calendars, weather‑based reschedules.

### K. Privacy, Security & Legal
34. **Data locality**: Must data stay on‑prem? Is cloud relay allowed? End‑to‑end encryption?
35. **Right to be forgotten**: How do we purge a child’s data upon request? Audit log retention policy?
36. **Abuse prevention**: Protections against surveillance or coercive control (e.g., hidden‑mode resources)?

### L. Governance & Admin
37. **Admins**: Who can create members, reset PINs, change privacy defaults, export data?
38. **Conflict logging**: Do we keep mediated negotiation records for transparency? For how long?
39. **House split/merge**: Family dissolves or merges—how do we migrate portions of data cleanly?

### M. Metrics & Success
40. **KPIs**: What will prove cognitive‑load reduction? (missed tasks ↓, mom’s mental checklist ↓, NPS ↑)
41. **Feedback loop**: In‑product micro‑surveys? Weekly “How did this week feel?” check‑ins?

### N. Internationalization
42. **Languages**: EN/NL at launch? Date formats, school holiday calendars, garbage pickup ICS by municipality?

### O. Hardware & Install
43. **Hub choice**: Tablet vs. Pi + display; PoE; wall‑mount constraints; always‑on power management.
44. **Auth at hub**: Per‑member quick switch (NFC/QR/PIN/emoji). Auto‑switch via face‑recognition or presence?

---

## 22) Decision Worksheets (fill these tables)
### 22.1 Privacy/Visibility Matrix
| Item Type | Children See | Teens See | Adults See | Owner Overrides? | Retention |
|---|---|---|---|---|---|
| Private event |  |  |  |  |  |
| Medical task |  |  |  |  |  |
| Homework |  |  |  |  |  |
| Chore assignments |  |  |  |  |  |

### 22.2 Fairness Settings
- Effort scale: 1–5 definition examples
- Weighting: time 40% / unpleasantness 30% / reliability 20% / training 10% (editable)
- Rolling window: ___ days; decay λ = ___

### 22.3 Notification Policy
- Quiet hours per member
- Escalation tree for criticals
- Max nudges/day by priority

### 22.4 Meal/Grocery Policy
- Pantry low‑threshold rules
- Substitution rules (brand→generic)
- Store routing (Aldi vs Jumbo vs market)

---

## 23) Master Prompt Template (to drive future design sprints)
> Copy this into ChatGPT and fill the bracketed fields.

**System / Context**
You are a product, UX, and family‑systems design partner. We are designing a local‑first family organizer with a kitchen hub, mobile apps, and limited on‑device voice. Privacy and fairness are first‑class.

**Key Inputs**
- Household profile: [members, ages, roles, custody rules]
- Privacy policy: [visibility matrix from §22.1]
- Fairness settings: [weights, window, constraints]
- Notification policy: [quiet hours, escalation, budgets]
- Meal/grocery policy: [dietary rules, stores, pantry rules]
- Integrations: [calendars, Home Assistant, email senders]
- Hardware: [hub device, mount, auth method]
- Languages: [EN/NL]
- Non‑goals: [list]

**Tasks for the model**
1. Propose weekly flows for Today/Week/Meals/Lists aligned to inputs.
2. Generate voice intent grammar (top 10) + error‑recovery dialogs.
3. Draft chore rotation algorithm config + 3 conflict scenarios with resolutions.
4. Produce push/alert schedule for a sample week under budget.
5. Output data schemas for Tasks/Events/Meals/Lists with privacy fields.
6. List risky edge cases and mitigations.

**Output Format**
- Sectioned Markdown with tables and bullet points. No code unless requested.

---

## 24) Edge Cases & Social Dynamics (challenge set)
- One parent disables notifications; how do we still ensure bins go out?
- Teen opts into privacy; a school trip fee is due—who gets told and how?
- Power outage / no internet weekend—what remains functional on the hub?
- Dishwasher chore undone 3×; fairness shows imbalance—what auto‑proposal appears?
- Guests staying 10 days—how do tasks and meals adapt temporarily?

---

## 25) Validation Plan Before Any Coding
1. **Paper prototypes** of the four tabs with your family; observe for 3 evenings.
2. **Cognitive walkthrough** using 5 scripted tasks (add milk, swap bins, plan dinner, log homework, add event).
3. **Policy simulation**: Run 2 weeks in a spreadsheet for rotation + notifications; measure conflict/resolution frequency.
4. **Privacy drill**: Try to access a teen’s private item from admin; verify policy outcomes.
5. **Data export rehearsal**: Populate sample data and perform full export → check for GDPR basics.

---

## 26) Open Assumptions to Test
- Load‑balancing improves perceived fairness more than gamified points.
- Local‑first with cloud relay meets convenience *and* privacy needs.
- Limited voice (10 intents) covers 80% of real kitchen commands.
- Glanceable hub reduces mental checklists by ≥30%.

---

## 27) Your Turn
Please answer §21 questions first (even terse bullets). Then fill the §22 worksheets. Once done, we’ll use §23 to generate the long, detailed master prompt and iterate.
