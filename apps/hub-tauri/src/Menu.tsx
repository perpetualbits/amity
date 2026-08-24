/* Menu.tsx — the weekly meal-planning strip.
 *
 * A Monday-first week strip (mirrors Week.tsx's week-navigation and
 * Monday-anchor approach, but with its own date helpers — Menu's meals come
 * from a separate endpoint with its own date-range shape, so there is no
 * `WeekResponse` to reuse here). Each day shows its planned dinner — dish
 * name plus cook, if recorded — or an empty affordance to plan one.
 *
 * This view is read-mostly apart from planning: tapping "+ Plan a meal" opens
 * a minimal, Capture-sized form (dish name, optional cook, optional
 * ingredient lines) inline under that day. There is no editing or deleting a
 * planned meal here — the service has no update endpoint for meals yet (see
 * amity-service's api/meal.rs module doc), so this view stays exactly as
 * form-light as the brief asks.
 */

import { createSignal, onMount, For, Show } from "solid-js";
import { createMeal, listMeals, type IngredientLine, type Meal } from "./api";
import { ensureMembersLoaded, members, memberById } from "./members";
import MemberPicker from "./MemberPicker";

export default function Menu() {
  // The Monday (YYYY-MM-DD) of the week currently displayed.
  const [weekStart, setWeekStart] = createSignal(mondayOf(todayISODate()));
  // The loaded meals for the displayed week (any slot; the strip filters to
  // dinner when rendering).
  const [meals, setMeals] = createSignal<Meal[]>([]);
  // True until the first fetch resolves, so we do not flash an empty strip.
  const [loading, setLoading] = createSignal(true);
  // Error message when the fetch or a mutation fails; null when there is none.
  const [error, setError] = createSignal<string | null>(null);
  // True while a mutation (plan a meal) is in flight — disables the form.
  const [busy, setBusy] = createSignal(false);
  // The date (YYYY-MM-DD) whose plan-a-meal form is open; null when none is.
  const [openFor, setOpenFor] = createSignal<string | null>(null);

  /** Load (or reload) the week starting on `monday`. */
  async function load(monday: string) {
    setError(null);
    try {
      const sunday = shiftDate(monday, 6);
      const result = await listMeals(monday, sunday);
      setMeals(result);
      setWeekStart(monday);
    } catch (err) {
      setError(typeof err === "string" ? err : "could not load menu");
    } finally {
      setLoading(false);
    }
  }

  // Load "this week" once when the view mounts, alongside the shared member
  // roster (idempotent — a no-op if another view already triggered it).
  onMount(() => {
    load(weekStart());
    ensureMembersLoaded();
  });

  /** Step to the previous or next week. */
  function shiftWeek(days: number) {
    load(shiftDate(weekStart(), days));
  }

  /** Return to the week containing today. */
  function thisWeek() {
    load(mondayOf(todayISODate()));
  }

  // Today's date, in the member's local time.
  const todayIso = todayISODate();

  /** The planned dinner(s) for a given date, in the order the service
   * returned them. Menu is scoped to the dinner slot — the slot households
   * overwhelmingly plan ahead of time (matching Today's own scoping, see
   * amity-service's build_meal_candidates doc comment). */
  const dinnersFor = (date: string) =>
    meals().filter((m) => m.date === date && m.slot === "dinner");

  return (
    <section class="menu-section" aria-label="Menu">
      {/* ── Navigation ───────────────────────────────────────────────────── */}
      <div class="week-nav">
        <button
          class="week-nav-btn"
          type="button"
          disabled={loading()}
          aria-label="Previous week"
          onClick={() => shiftWeek(-7)}
        >
          ‹ Prev
        </button>
        <button
          class="week-nav-btn week-nav-today"
          type="button"
          disabled={loading()}
          onClick={thisWeek}
        >
          This week
        </button>
        <button
          class="week-nav-btn"
          type="button"
          disabled={loading()}
          aria-label="Next week"
          onClick={() => shiftWeek(7)}
        >
          Next ›
        </button>
      </div>

      <Show when={error()}>
        <p class="capture-error" role="alert">
          {error()}
        </p>
      </Show>

      <Show when={!loading()}>
        <div class="menu-grid">
          <For each={weekDates(weekStart())}>
            {(date) => {
              const isToday = date === todayIso;
              const dinners = () => dinnersFor(date);

              return (
                <div class="menu-day" classList={{ "is-today": isToday }}>
                  <div class="menu-day-header">
                    <span class="menu-day-name">{dayName(date)}</span>
                    <span class="menu-day-date">{dayDateLabel(date)}</span>
                    <Show when={isToday}>
                      <span class="menu-day-badge">today</span>
                    </Show>
                  </div>

                  <Show
                    when={dinners().length > 0}
                    fallback={
                      <Show
                        when={openFor() === date}
                        fallback={
                          <button
                            class="menu-plan-toggle"
                            type="button"
                            onClick={() => setOpenFor(date)}
                          >
                            + Plan a meal
                          </button>
                        }
                      >
                        <PlanMealForm
                          date={date}
                          busy={busy()}
                          setBusy={setBusy}
                          onCreated={async () => {
                            setOpenFor(null);
                            await load(weekStart());
                          }}
                          onCancel={() => setOpenFor(null)}
                          onError={setError}
                        />
                      </Show>
                    }
                  >
                    <ul class="menu-dinner-list">
                      <For each={dinners()}>
                        {(meal) => {
                          // Resolve the cook once per row; a dangling id
                          // (no matching member) falls back to "—", never
                          // an error (Task 9 Slice 2 design decision).
                          const cook = () => memberById(members(), meal.cook);
                          return (
                            <li class="menu-dinner">
                              <span class="menu-dinner-name">{meal.name}</span>
                              <Show when={meal.cook}>
                                <span class="menu-dinner-cook">
                                  {" · "}
                                  <Show when={cook()?.color}>
                                    <span
                                      class={`member-dot member-dot-${cook()?.color}`}
                                      aria-hidden="true"
                                    />
                                  </Show>
                                  {cook()?.display_name ?? "—"}
                                </span>
                              </Show>
                            </li>
                          );
                        }}
                      </For>
                    </ul>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </section>
  );
}

/** The minimal plan-a-meal form: dish name, optional cook, optional
 * ingredient lines — kept as small as Capture's own form, not a form-heavy
 * planner (brief). */
function PlanMealForm(props: {
  date: string;
  busy: boolean;
  setBusy: (b: boolean) => void;
  onCreated: () => void | Promise<void>;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const [name, setName] = createSignal("");
  // The chosen cook's member id, or null for "no one".
  const [cook, setCook] = createSignal<string | null>(null);
  // Freetext, comma-separated ingredient lines, e.g. "tofu, rice, curry paste (2 tbsp)".
  const [ingredients, setIngredients] = createSignal("");

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const dishName = name().trim();
    if (!dishName) return;
    props.setBusy(true);
    try {
      const lines = parseIngredientLines(ingredients());
      await createMeal({
        name: dishName,
        date: props.date,
        cook: cook() ?? undefined,
        ingredientLines: lines.length > 0 ? lines : undefined,
      });
      await props.onCreated();
    } catch (err) {
      props.onError(typeof err === "string" ? err : "could not plan meal");
    } finally {
      props.setBusy(false);
    }
  }

  return (
    <form class="planmeal-form" onSubmit={handleSubmit}>
      <input
        class="capture-input"
        type="text"
        value={name()}
        onInput={(e) => setName(e.currentTarget.value)}
        placeholder="what's for dinner?"
        autocomplete="off"
        required
      />
      <div class="planmeal-cook-field">
        <span class="planmeal-cook-label">Cook</span>
        <MemberPicker members={members()} selected={cook()} onSelect={setCook} disabled={props.busy} />
      </div>
      <input
        class="capture-input"
        type="text"
        value={ingredients()}
        onInput={(e) => setIngredients(e.currentTarget.value)}
        placeholder="ingredients, comma separated — e.g. rice, tofu (200g)"
        autocomplete="off"
      />
      <div class="planmeal-actions">
        <button
          class="capture-submit planmeal-submit"
          type="submit"
          disabled={props.busy || !name().trim()}
        >
          Save
        </button>
        <button class="taskform-cancel" type="button" onClick={props.onCancel} disabled={props.busy}>
          Cancel
        </button>
      </div>
    </form>
  );
}

// ─── Ingredient parsing ─────────────────────────────────────────────────────

/** Parse a freetext "name, name (qty), …" string into ingredient lines. A
 * trailing "(…)" on an entry is taken as its quantity; blank entries are
 * dropped. */
function parseIngredientLines(text: string): IngredientLine[] {
  return text
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
    .map((entry) => {
      const match = entry.match(/^(.*?)\s*\(([^)]+)\)\s*$/);
      if (match) {
        return { name: match[1].trim(), qty: match[2].trim() };
      }
      return { name: entry };
    });
}

// ─── Date helpers ─────────────────────────────────────────────────────────
//
// Duplicated in spirit from Week.tsx (which duplicates them from nowhere —
// there is no shared date-utils module yet). Menu's meal endpoint takes an
// explicit from/to range rather than a single "any date in the week" anchor,
// so it computes its own Monday rather than reusing Week's server-resolved one.

/** Today's date in the viewer's local time, as `YYYY-MM-DD`. */
function todayISODate(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** The Monday (YYYY-MM-DD) of the week containing `dateStr`. Parsed and
 * re-formatted in UTC so the calendar date itself moves, unaffected by the
 * viewer's local offset. */
function mondayOf(dateStr: string): string {
  const d = new Date(`${dateStr}T00:00:00Z`);
  // getUTCDay(): Sunday=0..Saturday=6. Offset from Monday: 0 for Monday, 6 for Sunday.
  const offset = (d.getUTCDay() + 6) % 7;
  d.setUTCDate(d.getUTCDate() - offset);
  return formatISODate(d);
}

/** Shift a `YYYY-MM-DD` date string by `days` (may be negative). */
function shiftDate(dateStr: string, days: number): string {
  const d = new Date(`${dateStr}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return formatISODate(d);
}

/** Format a `Date`'s UTC calendar date as `YYYY-MM-DD`. */
function formatISODate(d: Date): string {
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** The 7 dates (YYYY-MM-DD) of the week starting on `monday`, in order. */
function weekDates(monday: string): string[] {
  return Array.from({ length: 7 }, (_, i) => shiftDate(monday, i));
}

/** Short weekday name for a `YYYY-MM-DD` date, e.g. "Mon". */
function dayName(dateStr: string): string {
  return new Date(`${dateStr}T00:00:00Z`).toLocaleDateString([], {
    weekday: "short",
    timeZone: "UTC",
  });
}

/** Short date label for a `YYYY-MM-DD` date, e.g. "18 Aug" — day-first, per
 * the Dutch convention (brief §16). */
function dayDateLabel(dateStr: string): string {
  return new Date(`${dateStr}T00:00:00Z`).toLocaleDateString([], {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  });
}
