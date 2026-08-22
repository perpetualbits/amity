/* App.tsx — the hub shell.
 *
 * Five views behind a small segmented control: Today (the ranked "what's on
 * today" surface), Week (the 7-day grid), Capture (the unified inbox), Menu
 * (the weekly meal-planning strip), and Groceries (the current list). Today
 * leads, because the hub's job at rest is to show the day calmly; the rest
 * are one tap away.
 *
 * No router — a single `view` signal switches the rendered component, matching
 * the ratified Task 3 plan (a segmented control, no routing library). Five
 * tabs are wider than the original three; rather than shrinking the touch
 * targets to force a fit, `.viewswitch` wraps onto a second row on a narrow
 * window (see style.css) — every tab keeps its full 60px target regardless of
 * window width.
 */

import { createSignal, Switch, Match } from "solid-js";
import Today from "./Today";
import Week from "./Week";
import Capture from "./Capture";
import Menu from "./Menu";
import Groceries from "./Groceries";

type View = "today" | "week" | "capture" | "menu" | "groceries";

export default function App() {
  // Which view is active; Today is the default at-rest surface.
  const [view, setView] = createSignal<View>("today");

  return (
    <main class="hub">
      {/* ── View switch ─────────────────────────────────────────────────── */}
      <nav class="viewswitch" role="tablist" aria-label="Views">
        <button
          class="seg"
          classList={{ active: view() === "today" }}
          role="tab"
          aria-selected={view() === "today"}
          type="button"
          onClick={() => setView("today")}
        >
          Today
        </button>
        <button
          class="seg"
          classList={{ active: view() === "week" }}
          role="tab"
          aria-selected={view() === "week"}
          type="button"
          onClick={() => setView("week")}
        >
          Week
        </button>
        <button
          class="seg"
          classList={{ active: view() === "capture" }}
          role="tab"
          aria-selected={view() === "capture"}
          type="button"
          onClick={() => setView("capture")}
        >
          Capture
        </button>
        <button
          class="seg"
          classList={{ active: view() === "menu" }}
          role="tab"
          aria-selected={view() === "menu"}
          type="button"
          onClick={() => setView("menu")}
        >
          Menu
        </button>
        <button
          class="seg"
          classList={{ active: view() === "groceries" }}
          role="tab"
          aria-selected={view() === "groceries"}
          type="button"
          onClick={() => setView("groceries")}
        >
          Groceries
        </button>
      </nav>

      {/* Render the active view. Each owns its own data fetching. */}
      <Switch>
        <Match when={view() === "today"}>
          <Today />
        </Match>
        <Match when={view() === "week"}>
          <Week />
        </Match>
        <Match when={view() === "capture"}>
          <Capture />
        </Match>
        <Match when={view() === "menu"}>
          <Menu />
        </Match>
        <Match when={view() === "groceries"}>
          <Groceries />
        </Match>
      </Switch>
    </main>
  );
}
