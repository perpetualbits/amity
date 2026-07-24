/* Capture.tsx — the unified inbox capture view.
 *
 * A capture form and a recent-items list. This is the system's first promise:
 * one free-text intake for any thought. The feedback is deliberately quiet —
 * the item appears in the list; there is no toast, no animation (brief §3).
 *
 * Extracted unchanged in behaviour from the original single-screen App, now one
 * of the two views behind the Capture/Today switch.
 */

import { createSignal, onMount, For, Show } from "solid-js";
import { captureInboxItem, listRecentInbox, formatWhen, type InboxItem } from "./api";

export default function Capture() {
  // The text currently typed in the capture input.
  const [captureText, setCaptureText] = createSignal("");
  // The recent inbox items shown below the form.
  const [items, setItems] = createSignal<InboxItem[]>([]);
  // Error message when capture fails; null when there is none.
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  // True while a capture request is in flight (disables the submit button).
  const [submitting, setSubmitting] = createSignal(false);

  /** Load the most recent inbox items from the service. */
  async function loadRecent() {
    try {
      setItems(await listRecentInbox(20));
    } catch (err) {
      // A failed refresh is silent — the existing list stays visible.
      console.error("list_recent_inbox failed:", err);
    }
  }

  /** Submit the capture form. */
  async function handleSubmit(e: Event) {
    e.preventDefault();
    const text = captureText().trim();
    if (!text) return;
    setSubmitting(true);
    setErrorMessage(null);
    try {
      await captureInboxItem(text);
      // Clear the input on success and refresh the list.
      setCaptureText("");
      await loadRecent();
    } catch (err) {
      setErrorMessage(typeof err === "string" ? err : "capture failed");
    } finally {
      setSubmitting(false);
    }
  }

  // Load the list once when the view mounts.
  onMount(loadRecent);

  return (
    <>
      {/* ── Capture form ────────────────────────────────────────────────── */}
      <section class="capture-section" aria-label="Capture">
        <form class="capture-form" onSubmit={handleSubmit}>
          <label class="capture-label" for="capture-input">
            What's on your mind?
          </label>
          <div class="capture-row">
            <input
              id="capture-input"
              class="capture-input"
              type="text"
              value={captureText()}
              onInput={(e) => setCaptureText(e.currentTarget.value)}
              placeholder="type a thought…"
              autocomplete="off"
              required
              disabled={submitting()}
            />
            {/* 60×60px minimum touch target (brief §12.4). */}
            <button
              class="capture-submit"
              type="submit"
              disabled={submitting() || !captureText().trim()}
              aria-label="Capture"
            >
              ↵
            </button>
          </div>
          <Show when={errorMessage()}>
            <p class="capture-error" role="alert">
              {errorMessage()}
            </p>
          </Show>
        </form>
      </section>

      {/* ── Recent items ────────────────────────────────────────────────── */}
      <section class="recent-section" aria-label="Recent captures">
        <Show
          when={items().length > 0}
          fallback={<p class="empty-state">nothing captured yet</p>}
        >
          <ol class="item-list" reversed>
            <For each={items()}>
              {(item) => (
                <li class="item">
                  <span class="item-text">{item.raw_text}</span>
                  <time class="item-time" dateTime={item.captured_at}>
                    {formatWhen(item.captured_at)}
                  </time>
                </li>
              )}
            </For>
          </ol>
        </Show>
      </section>
    </>
  );
}
