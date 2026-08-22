/* Today.tsx — the Today view and the task-capture form.
 *
 * Renders the ranked "what's on today" list from the surfacing endpoint. When
 * nothing is due it shows a calm "nothing today" and means it — the empty state
 * is a designed state (brief §3, §11.5). Each item offers the two affordances
 * the hub needs: mark it done, and reassign it (scaffolding until real members).
 *
 * A `kind: "meal"` item (P2 Slice 3's third surfaced kind, dinner only) is
 * purely informational: a distinct fork-and-knife marker, the dish name, and
 * whether a cook is assigned — no Done, no Reassign. It has no lifecycle to
 * settle (see amity-service's build_meal_candidates doc comment), so it is
 * quiet exactly like an event, just with its own glyph.
 *
 * An "Add a task" toggle reveals a structured capture form. Recurrence is chosen
 * from a small set of presets rather than exposing raw RRULE, matching the
 * ratified Task 3 plan.
 */

import { createSignal, onMount, For, Show } from "solid-js";
import {
  surfacingToday,
  completeTask,
  changeAssignee,
  createTask,
  formatWhen,
  PLACEHOLDER_MEMBER,
  type SurfacedItem,
} from "./api";

export default function Today() {
  // The surfaced items for the day.
  const [items, setItems] = createSignal<SurfacedItem[]>([]);
  // Whether anything crossed the "surface today" threshold.
  const [hasSurfaced, setHasSurfaced] = createSignal(false);
  // True until the first fetch resolves, so we do not flash the empty state.
  const [loading, setLoading] = createSignal(true);
  // Error message when the fetch fails; null when there is none.
  const [error, setError] = createSignal<string | null>(null);
  // True while a mutation (done/reassign/create) is in flight — disables actions.
  const [busy, setBusy] = createSignal(false);
  // Whether the add-a-task form is open.
  const [formOpen, setFormOpen] = createSignal(false);

  /** Load (or reload) the Today view from the service. */
  async function load() {
    setError(null);
    try {
      const today = await surfacingToday();
      setItems(today.items);
      setHasSurfaced(today.has_surfaced);
    } catch (err) {
      setError(typeof err === "string" ? err : "could not load today");
    } finally {
      setLoading(false);
    }
  }

  /** Mark a surfaced item done, then reload. */
  async function markDone(item: SurfacedItem) {
    setBusy(true);
    try {
      // The instance date is the calendar date of the item's salient instant.
      await completeTask(item.source_id, item.at.slice(0, 10));
      await load();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not mark done");
    } finally {
      setBusy(false);
    }
  }

  /** Reassign an item. With one member this is scaffolding — it re-affirms the
   * placeholder assignee — but the call is real end-to-end. */
  async function reassign(item: SurfacedItem) {
    setBusy(true);
    try {
      await changeAssignee(item.source_id, PLACEHOLDER_MEMBER);
      await load();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not reassign");
    } finally {
      setBusy(false);
    }
  }

  // Load once when the view mounts.
  onMount(load);

  /** True for a task — the only kind that carries the done / reassign actions.
   * An event simply occurs: it is not "whose turn", so it offers no actions. */
  const isTask = (item: SurfacedItem) => item.kind === "task";

  /** True for a meal — informational only, like an event, but with its own
   * marker and no lifecycle actions. */
  const isMeal = (item: SurfacedItem) => item.kind === "meal";

  /** The kind marker glyph: an open ring for a task, a filled diamond for an
   * event, a fork-and-knife for a meal. A shape (not colour alone) carries
   * the distinction (brief §12.4); the aria-label on the span names the kind
   * so it is not conveyed by glyph alone. */
  const kindGlyph = (item: SurfacedItem) => (isTask(item) ? "○" : isMeal(item) ? "🍴" : "◆");

  /** The time label for an item: an all-day event reads "all day" (its instant
   * is midnight, which would otherwise show as 00:00); everything else shows
   * its salient time. */
  const whenLabel = (item: SurfacedItem) => (item.all_day ? "all day" : formatWhen(item.at));

  return (
    <>
      {/* ── The day's items ─────────────────────────────────────────────── */}
      <section class="today-section" aria-label="Today">
        <Show when={error()}>
          <p class="capture-error" role="alert">
            {error()}
          </p>
        </Show>

        <Show when={!loading()}>
          <Show
            when={hasSurfaced() && items().length > 0}
            fallback={<p class="empty-state">nothing today</p>}
          >
            <ol class="today-list">
              <For each={items()}>
                {(item) => (
                  <li
                    class="today-item"
                    classList={{ "is-event": item.kind === "event", "is-meal": isMeal(item) }}
                  >
                    {/* Kind marker: a shape, not colour alone (brief §12.4). A
                        task shows an open ring (it can be checked off); an event
                        shows a filled diamond (it simply occurs); a meal shows a
                        fork and knife (also purely informational). The aria-label
                        names the kind so it is not conveyed by glyph alone. */}
                    <span
                      class="today-kind"
                      aria-label={isTask(item) ? "task" : isMeal(item) ? "meal" : "event"}
                    >
                      {kindGlyph(item)}
                    </span>
                    <div class="today-main">
                      <span class="today-title">{item.title}</span>
                      <span class="today-meta">
                        <time class="item-time" dateTime={item.at}>
                          {whenLabel(item)}
                        </time>
                        {/* Overdue is shown as information, never a red badge
                            or a count of how late it is (brief §3, §11). */}
                        <Show when={item.overdue}>
                          <span class="today-overdue"> · due earlier</span>
                        </Show>
                        {/* The meal's cook, when one is assigned — informational
                            only, no name resolution yet (there is no member
                            registry to look one up in). */}
                        <Show when={isMeal(item) && item.current_assignee_id}>
                          <span class="today-cook"> · cook assigned</span>
                        </Show>
                      </span>
                    </div>
                    {/* Only tasks carry actions. An event or meal has no "done"
                        — it is not whose turn, it just happens — so its row is
                        quiet. */}
                    <Show when={isTask(item)}>
                      <div class="today-actions">
                        <button
                          class="today-done"
                          type="button"
                          disabled={busy()}
                          aria-label={`Mark "${item.title}" done`}
                          onClick={() => markDone(item)}
                        >
                          Done
                        </button>
                        <button
                          class="today-reassign"
                          type="button"
                          disabled={busy()}
                          aria-label={`Reassign "${item.title}"`}
                          onClick={() => reassign(item)}
                        >
                          Reassign
                        </button>
                      </div>
                    </Show>
                  </li>
                )}
              </For>
            </ol>
          </Show>
        </Show>
      </section>

      {/* ── Add a task ──────────────────────────────────────────────────── */}
      <section class="addtask-section" aria-label="Add a task">
        <Show
          when={formOpen()}
          fallback={
            <button class="addtask-toggle" type="button" onClick={() => setFormOpen(true)}>
              + Add a task
            </button>
          }
        >
          <TaskForm
            busy={busy()}
            setBusy={setBusy}
            onCreated={async () => {
              setFormOpen(false);
              await load();
            }}
            onCancel={() => setFormOpen(false)}
          />
        </Show>
      </section>
    </>
  );
}

/** The structured task-capture form. */
function TaskForm(props: {
  busy: boolean;
  setBusy: (b: boolean) => void;
  onCreated: () => void | Promise<void>;
  onCancel: () => void;
}) {
  // Form fields.
  const [title, setTitle] = createSignal("");
  const [notes, setNotes] = createSignal("");
  const [dueBy, setDueBy] = createSignal(""); // YYYY-MM-DD from the date input
  const [tags, setTags] = createSignal("");
  // Recurrence preset and its two dependent inputs.
  const [recurrence, setRecurrence] = createSignal<"once" | "daily" | "weekly" | "monthly">("once");
  const [weekday, setWeekday] = createSignal("MO");
  const [monthday, setMonthday] = createSignal(1);
  // Field-level error (e.g. an empty title rejected by the service).
  const [error, setError] = createSignal<string | null>(null);

  /** Turn the preset into an RRULE string, or null for a one-shot task. */
  function buildRrule(): string | null {
    switch (recurrence()) {
      case "daily":
        return "FREQ=DAILY";
      case "weekly":
        return `FREQ=WEEKLY;BYDAY=${weekday()}`;
      case "monthly":
        return `FREQ=MONTHLY;BYMONTHDAY=${monthday()}`;
      default:
        return null; // "once" — no recurrence
    }
  }

  /** Submit the form: build the request and create the task. */
  async function handleSubmit(e: Event) {
    e.preventDefault();
    const t = title().trim();
    if (!t) return;
    props.setBusy(true);
    setError(null);
    try {
      const rrule = buildRrule();
      const tagList = tags()
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      await createTask({
        title: t,
        notes: notes().trim() || undefined,
        // A date input gives YYYY-MM-DD; the service wants RFC 3339, so anchor
        // it at midday UTC (safe from date-boundary drift at NL offsets).
        dueBy: dueBy() ? `${dueBy()}T12:00:00Z` : undefined,
        recurrenceRrule: rrule ?? undefined,
        recurrenceTimezone: rrule ? "Europe/Amsterdam" : undefined,
        tags: tagList.length > 0 ? tagList : undefined,
      });
      await props.onCreated();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not create task");
    } finally {
      props.setBusy(false);
    }
  }

  return (
    <form class="taskform" onSubmit={handleSubmit}>
      <label class="taskform-label" for="task-title">
        New task
      </label>
      <input
        id="task-title"
        class="capture-input"
        type="text"
        value={title()}
        onInput={(e) => setTitle(e.currentTarget.value)}
        placeholder="what needs doing?"
        autocomplete="off"
        required
      />

      <input
        class="capture-input taskform-notes"
        type="text"
        value={notes()}
        onInput={(e) => setNotes(e.currentTarget.value)}
        placeholder="notes (optional)"
        autocomplete="off"
      />

      <div class="taskform-row">
        <label class="taskform-field">
          <span>Due</span>
          <input
            class="capture-input"
            type="date"
            value={dueBy()}
            onInput={(e) => setDueBy(e.currentTarget.value)}
          />
        </label>

        <label class="taskform-field">
          <span>Repeat</span>
          <select
            class="capture-input"
            value={recurrence()}
            onInput={(e) => setRecurrence(e.currentTarget.value as ReturnType<typeof recurrence>)}
          >
            <option value="once">once</option>
            <option value="daily">daily</option>
            <option value="weekly">weekly on…</option>
            <option value="monthly">monthly on…</option>
          </select>
        </label>
      </div>

      {/* The weekly and monthly presets each reveal one dependent input. */}
      <Show when={recurrence() === "weekly"}>
        <label class="taskform-field">
          <span>Weekday</span>
          <select
            class="capture-input"
            value={weekday()}
            onInput={(e) => setWeekday(e.currentTarget.value)}
          >
            <option value="MO">Monday</option>
            <option value="TU">Tuesday</option>
            <option value="WE">Wednesday</option>
            <option value="TH">Thursday</option>
            <option value="FR">Friday</option>
            <option value="SA">Saturday</option>
            <option value="SU">Sunday</option>
          </select>
        </label>
      </Show>
      <Show when={recurrence() === "monthly"}>
        <label class="taskform-field">
          <span>Day of month</span>
          <input
            class="capture-input"
            type="number"
            min="1"
            max="28"
            value={monthday()}
            onInput={(e) => setMonthday(Number(e.currentTarget.value))}
          />
        </label>
      </Show>

      <input
        class="capture-input"
        type="text"
        value={tags()}
        onInput={(e) => setTags(e.currentTarget.value)}
        placeholder="tags, comma separated (optional)"
        autocomplete="off"
      />

      <Show when={error()}>
        <p class="capture-error" role="alert">
          {error()}
        </p>
      </Show>

      <div class="taskform-actions">
        <button class="capture-submit taskform-submit" type="submit" disabled={props.busy || !title().trim()}>
          Add task
        </button>
        <button class="taskform-cancel" type="button" onClick={props.onCancel} disabled={props.busy}>
          Cancel
        </button>
      </div>
    </form>
  );
}
