/* Week.tsx — the 7-day grid view.
 *
 * Renders `GET /api/v1/week` as a Monday-first 7-column grid (landscape) that
 * collapses to a single stacked column on narrow viewports (portrait), via a
 * CSS media query — see the "Week view" section of style.css. No JS layout
 * toggle: the grid switches automatically as the window's aspect changes,
 * which matches how the hub's window is actually resized/rotated.
 *
 * This view is read-mostly (brief §6): each item shows its kind, time,
 * annotation, and reschedule marker, but there is no editing, dragging, or
 * creating here — that all goes through Capture. Tapping an item is
 * intentionally inert; only the per-day "and N more" affordance and the
 * prev/this-week/next navigation are interactive.
 *
 * The wire (`SurfacedItem`) does not distinguish an external-calendar event
 * from a native one — both are simply `kind === "event"`. The kind marker
 * below is therefore the same generic event diamond Today uses; there is no
 * additional "external" marker because the data to draw one does not exist
 * yet at this endpoint.
 */

import { createSignal, onMount, For, Show } from "solid-js";
import { week, type SurfacedItem, type WeekResponse } from "./api";

/** Items shown per day before the "and N more" affordance takes over. Fixed
 * rather than measured — the brief asks not to shrink text to fit, and a
 * fixed cap keeps every day's card the same height regardless of content. */
const ITEMS_VISIBLE = 4;

export default function Week() {
  // The current week's data; null until the first load resolves.
  const [data, setData] = createSignal<WeekResponse | null>(null);
  // True until the first fetch resolves, so we do not flash an empty grid.
  const [loading, setLoading] = createSignal(true);
  // Error message when the fetch fails; null when there is none.
  const [error, setError] = createSignal<string | null>(null);
  // Dates (YYYY-MM-DD) whose "and N more" has been expanded.
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());

  /** Load (or reload) the week containing `start` (any date in it); absent
   * means "this week", resolved by the service from its own clock. */
  async function load(start?: string) {
    setError(null);
    try {
      const result = await week(start);
      setData(result);
    } catch (err) {
      setError(typeof err === "string" ? err : "could not load week");
    } finally {
      setLoading(false);
    }
  }

  // Load "this week" once when the view mounts.
  onMount(() => load());

  /** Step to the previous or next week, relative to the loaded week's Monday. */
  function shiftWeek(days: number) {
    const current = data();
    if (!current) return;
    load(shiftDate(current.start, days));
  }

  /** Return to the week containing today. */
  function thisWeek() {
    load();
  }

  /** Toggle a day's "and N more" expansion. */
  function toggleExpanded(date: string) {
    const next = new Set(expanded());
    if (next.has(date)) {
      next.delete(date);
    } else {
      next.add(date);
    }
    setExpanded(next);
  }

  // Today's date, in the member's local time — the column this marks is the
  // day they are actually standing in, not the server's UTC day.
  const todayIso = todayISODate();

  /** True for a task — used for the kind marker, matching Today's convention. */
  const isTask = (item: SurfacedItem) => item.kind === "task";

  /** The time label for an item: all-day reads "all day"; a timed item always
   * shows its clock time (24h). Week must NOT reuse api's `formatWhen`, which
   * collapses non-today items to a date — every column here is a different day,
   * so the clock time is exactly what the reader needs, and the date already
   * lives in the column header. */
  const whenLabel = (item: SurfacedItem) => (item.all_day ? "all day" : formatTime(item.at));

  return (
    <section class="week-section" aria-label="Week">
      {/* ── Navigation ───────────────────────────────────────────────────── */}
      <div class="week-nav">
        <button
          class="week-nav-btn"
          type="button"
          disabled={loading() || !data()}
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
          disabled={loading() || !data()}
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

      <Show when={!loading() && data()}>
        <div class="week-grid">
          <For each={data()!.days}>
            {(day) => {
              const isToday = day.date === todayIso;
              const isOpen = () => expanded().has(day.date);
              const visibleItems = () => (isOpen() ? day.items : day.items.slice(0, ITEMS_VISIBLE));
              const hiddenCount = () => day.items.length - ITEMS_VISIBLE;

              return (
                <div class="week-day" classList={{ "is-today": isToday }}>
                  {/* Today's column is marked by more than colour: a bold
                      label, not just a tint (brief §12.4). */}
                  <div class="week-day-header">
                    <span class="week-day-name">{dayName(day.date)}</span>
                    <span class="week-day-date">{dayDateLabel(day.date)}</span>
                    <Show when={isToday}>
                      <span class="week-day-badge">today</span>
                    </Show>
                  </div>

                  <Show when={day.items.length > 0} fallback={<p class="week-day-empty">—</p>}>
                    <ol class="week-day-list">
                      <For each={visibleItems()}>
                        {(item) => (
                          <li class="week-item" classList={{ "is-event": !isTask(item) }}>
                            <span
                              class="week-item-kind"
                              aria-label={isTask(item) ? "task" : "event"}
                            >
                              {isTask(item) ? "○" : "◆"}
                            </span>
                            <div class="week-item-main">
                              <span class="week-item-title">{item.title}</span>
                              <span class="week-item-meta">
                                <time class="item-time" dateTime={item.at}>
                                  {whenLabel(item)}
                                </time>
                                <Show when={item.overdue}>
                                  <span class="week-item-overdue"> · due earlier</span>
                                </Show>
                                <Show when={item.rescheduled}>
                                  <span class="week-item-rescheduled" aria-label="rescheduled">
                                    {" "}
                                    · ↻ moved
                                  </span>
                                </Show>
                              </span>
                              <Show when={item.annotation}>
                                <span class="week-item-annotation">{item.annotation}</span>
                              </Show>
                            </div>
                          </li>
                        )}
                      </For>
                    </ol>

                    <Show when={!isOpen() && hiddenCount() > 0}>
                      <button
                        class="week-more"
                        type="button"
                        onClick={() => toggleExpanded(day.date)}
                      >
                        and {hiddenCount()} more
                      </button>
                    </Show>
                    <Show when={isOpen() && day.items.length > ITEMS_VISIBLE}>
                      <button
                        class="week-more"
                        type="button"
                        onClick={() => toggleExpanded(day.date)}
                      >
                        show less
                      </button>
                    </Show>
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

// ─── Date helpers ─────────────────────────────────────────────────────────

/** Shift a `YYYY-MM-DD` date string by `days` (may be negative). Parsed and
 * re-formatted in UTC so the calendar date itself moves, unaffected by the
 * viewer's local offset. */
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

/** Today's date in the viewer's local time, as `YYYY-MM-DD` — the wall-clock
 * day they are standing in, not the server's UTC day. */
function todayISODate(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** Clock time (24h, e.g. "09:00") for an RFC 3339 instant, shown in home/local
 * time — always, with no same-day branch (that is what `formatWhen` gets wrong
 * for the Week grid). Per the Dutch 24-hour convention (brief §16). */
function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** Short weekday name for a `YYYY-MM-DD` date, e.g. "Mon". Rendered in UTC so
 * the label matches the calendar date the service assigned, regardless of
 * the viewer's offset. */
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
