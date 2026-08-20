/* App.tsx — the hub shell.
 *
 * Three views behind a small segmented control: Today (the ranked "what's on
 * today" surface), Week (the 7-day grid), and Capture (the unified inbox).
 * Today leads, because the hub's job at rest is to show the day calmly;
 * Week and Capture are one tap away.
 *
 * No router — a single `view` signal switches the rendered component, matching
 * the ratified Task 3 plan (a segmented control, no routing library).
 */

import { createSignal, Switch, Match } from "solid-js";
import Today from "./Today";
import Week from "./Week";
import Capture from "./Capture";

type View = "today" | "week" | "capture";

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
      </Switch>
    </main>
  );
}
