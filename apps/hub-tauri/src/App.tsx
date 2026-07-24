/* App.tsx — the hub shell.
 *
 * Two views behind a small segmented control: Today (the ranked "what's on
 * today" surface) and Capture (the unified inbox). Today leads, because the
 * hub's job at rest is to show the day calmly; Capture is one tap away.
 *
 * No router — a single `view` signal switches the rendered component, matching
 * the ratified Task 3 plan (a two-item segmented control, no routing library).
 */

import { createSignal, Show } from "solid-js";
import Today from "./Today";
import Capture from "./Capture";

type View = "today" | "capture";

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
      <Show when={view() === "today"} fallback={<Capture />}>
        <Today />
      </Show>
    </main>
  );
}
