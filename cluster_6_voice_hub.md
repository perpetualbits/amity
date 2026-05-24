## Cluster 6 — Voice & Hub UX

*Answers §21I (questions 28–30). Replaces §6 (Voice Intents) and parts of §4 (Today/Week/Meals/Lists interaction details). Establishes a categorical project commitment about microphone use.*

### 1. Categorical commitment: the kitchen is a safe place

This is the most important paragraph in this cluster. Everything else follows from it.

**The system never listens unless explicitly invoked.** No always-on microphone, no wake-word hotword running 24/7, no ambient audio buffering. Push-to-talk only — a physical or on-screen button activates listening for the duration of a single utterance, then the microphone disengages.

This is not a configurable setting. It is a categorical commitment of the project. Future requests to enable always-listening modes — whether for convenience, for partner integration, or for any other reason — are rejected by definition.

The reasoning is product-philosophical, not technical. Phones and assistants in many households already listen continuously, and the inevitable downstream consequence is advertising and third-party data exposure. A household organizer that aggregates pantry contents, meal plans, presence schedules, and family dynamics is the most commercially valuable surveillance target in the home. The only way to ensure the system never becomes that target is to structurally prevent it from being one. The kitchen is a safe place, and the system enforces that by what it cannot do, not by what it promises.

This commitment also closes a related door: no telemetry that exposes data to third parties, no data flow that could become an advertising surface, no partnerships that involve data in any direction. These are not policy choices that could be revisited; they are project boundaries.

### 2. Voice activation: push-to-talk

**Hub.** A clear, large button on the *Today* view activates voice. The microphone listens while the button is held (or for a fixed window after a single tap, configurable per user). Visual feedback shows listening state — a soft pulse, not a recording-light style indicator. When the user finishes speaking, the microphone disengages immediately. No buffering, no ambient capture.

**Mobile.** Same model — explicit invocation, no background listening. The mobile app exposes a voice button on the quick-capture surface.

**On-device transcription.** The captured audio is transcribed locally on the hub (or on the phone for mobile capture). The audio clip is processed and discarded; only the transcribed text persists, briefly, while the intent is being parsed. The text is then either turned into an action (a Task, a list item, an event) or — if recognition fails — dropped into the inbox as an untriaged note.

**Audio leaves the device only if the user explicitly opts in to a cloud-fallback for higher accuracy.** This is opt-in per device, never default, and the user is told clearly what the trade-off is. The system works fine without it.

### 3. Voice intents — the top 10

A short, sharply-scoped intent list outperforms a long, fuzzy one in actual use. More intents means users can't remember what works, try natural phrasings, and accumulate failure experiences that erode confidence in the channel.

Five of the ten intents are *capture*, not query. Voice's primary value is getting things *into* the system with minimal friction (hands full, cooking, remembering something passing through). Reading information back is secondary.

| # | Intent | Example utterance | Action |
|---|--------|------------------|--------|
| 1 | Add to groceries | "Add milk to groceries." | Creates `GroceryItem`. |
| 2 | Read meal plan | "What's for dinner?" / "What's for dinner Friday?" | Reads Meal label for the slot. |
| 3 | Add to use-first | "Add asparagus to use-first." | Creates `UseFirstItem`. |
| 4 | Add task | "Add task wash bikes for Saturday." / "Add task for Sam, dentist Wednesday at 14:30." | Creates a Task with member and dueBy when stated. |
| 5 | Note | "Note: ask Anneke about the holiday." | Creates an `InboxItem`, untriaged. |
| 6 | Look up event | "When is the school trip?" | Searches Events by partial title, reads first match. |
| 7 | Who's on the chore | "Who is doing the bins this week?" | Reads `currentAssigneeId` for the next instance. |
| 8 | Mark done | "Mark dishwasher done." | Writes a `CompletionLog` entry. |
| 9 | Snooze | "Snooze the dentist reminder to tomorrow." | Defers a Today item. |
| 10 | Cancel | "Cancel." | Aborts the current interaction. |

**The Note intent is the safety net.** When in doubt, the user dumps a thought without thinking about category. The inbox catches it. The user does not have to know which intent matches their thought — they can just speak. This absorbs a long tail of edge cases that would otherwise be voice failures.

### 4. Error recovery

Voice without good error recovery is worse than no voice. The system follows three rules:

1. **No silent failures.** Every utterance produces a confirmed action, a clarifying question, or an inbox capture. The system never simply does nothing.
2. **One follow-up, then fallback.** If recognition is ambiguous, the system asks one short follow-up ("Add to groceries or use-first?"). If still ambiguous after one round, the transcribed text is dropped into the inbox with a brief confirmation: "I've added that as a note for you to check."
3. **Confirmation on completion.** Every successful action gets a brief audio + visual confirmation. "Milk added to groceries." Short, no flourish.

The inbox-fallback is essential because it prevents voice from being a high-stakes interaction. If the system can't quite parse what was said, no problem — it lands somewhere the user can find later. This makes voice feel safe to use even when the user isn't sure their phrasing is "right".

### 5. Accessibility

These are commitments, not aspirations.

**Typography.** A single typeface across the entire system, sans-serif, with strong dyslexia accessibility. **Atkinson Hyperlegible** is the recommended choice (designed for low-vision users, readable at distance, freely licensed). Resist "fun" or playful fonts anywhere, including Kid Mode — children with reading difficulties suffer most from decorative typography.

**Type sizing.** The hub is read at conversational distance from across the kitchen, not arm's length. Body text on the hub is sized for ~1m comfortable reading. The *Today* view's primary items should be legible without leaning in.

**Touch targets.** Minimum 60×60 pixels on the hub for any interactive element; larger (80×80+) for primary actions. The hub is touched by people with wet hands, floury hands, kids' small hands, and people moving past quickly.

**Colour.** Colour never carries information alone. The LED ambient state uses warmth and pulsing, not red/green semantics. The UI uses position, text labels, and shape to convey state; colour augments. The system passes WCAG AA contrast at minimum, AAA where feasible.

**Languages.** EN and NL at launch. Per-member language preference, not per-household — mixed-language households are common. Date formats follow Dutch convention (day-first, 24-hour) by default; per-user override possible.

**NL voice caveat (honest scoping).** On-device transcription models for Dutch are currently less reliable than for English. Accent and dialect variation matter. The MVP accepts this — NL voice quality may be lower than EN voice quality, and the inbox-fallback path ensures degraded recognition still produces useful results rather than failures.

### 6. Hub authentication and member switching

The hub is a shared device. Members switch quickly without ceremony:

- **NFC tap.** Each member has an NFC tag (a card, a sticker on the back of their phone). Tap on the hub edge → switch context. Fastest.
- **PIN or emoji code.** Short, memorable per-member entry. Kids get an emoji sequence (more memorable than digits at age 7).
- **No facial recognition, no biometric capture by the system.** This is consistent with §1's commitment — biometric capture in the kitchen is a richer surveillance vector than audio. Not categorically required to refuse, but defaulted out of MVP for the same reasons.

A member's session times out after a configurable idle period (default 5 minutes) and the hub returns to the shared *Today* view (which shows household-visible items only). Anything private requires re-authentication.

### 7. What changed from §6 and §4

- Replaced: §6 wake-phrase configurability and on-device hotword. The hotword/always-listening model is **categorically removed**. Push-to-talk only.
- Replaced: §6's open-ended intent list. Defined a closed top-10 with mechanical scoping; `Note` is the safety-net catch-all.
- Added: explicit error recovery rules (no silent failures; one follow-up then inbox fallback; confirmation on completion).
- Added: explicit accessibility commitments (Atkinson Hyperlegible, target sizes, contrast, per-member language).
- Added: NL voice quality caveat — honest scoping rather than overpromising.
- Added: session timeout and shared-state default for the hub.
- Reframed: §1 is now a categorical project commitment, not a setting.
