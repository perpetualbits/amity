/* members.ts — the shared member registry resource for the hub.
 *
 * Every view that needs to resolve a person id to a name or render a member
 * picker reads from this one module-level store rather than fetching the
 * roster itself. `ensureMembersLoaded` is idempotent and safe to call from
 * every view's `onMount` — the first caller triggers the fetch, later
 * callers (and view re-mounts as the segmented control switches) reuse the
 * same in-memory list.
 *
 * Client-side resolution only (Task 9 Slice 2 design decision): the service
 * response shapes are unchanged, so a task's `current_assignee_id` or a
 * meal's `cook` is still a bare UUID on the wire. A dangling id — no
 * matching member row, e.g. old seed data predating the registry — resolves
 * to a neutral "—", never an error.
 */

import { createSignal } from "solid-js";
import { listMembers, type Member } from "./api";

// The current roster, reactive. Starts empty; `ensureMembersLoaded` fills it.
const [members, setMembers] = createSignal<Member[]>([]);

// Guards against re-fetching on every view mount. Reset to null on failure
// so a transient error (e.g. the service not up yet) can be retried the next
// time a view mounts, rather than leaving the roster permanently empty.
let loadPromise: Promise<void> | null = null;

/** Ensure the member roster has been fetched at least once. */
export function ensureMembersLoaded(): void {
  if (loadPromise) return;
  loadPromise = listMembers()
    .then((list) => {
      setMembers(list);
    })
    .catch(() => {
      // Leave the roster as-is (likely empty): name resolution falls back to
      // "—" and the picker degrades to just "no one" — never an error.
      loadPromise = null;
    });
}

export { members };

/** Resolve a member id to its display name. Returns "—" for an undefined or
 * dangling (unresolved) id — never throws, never renders an error. */
export function memberName(list: Member[], id: string | undefined | null): string {
  if (!id) return "—";
  const found = list.find((m) => m.id === id);
  return found ? found.display_name : "—";
}

/** Resolve a member id to its full record, for rendering a colour dot or
 * initial. Returns undefined for an undefined or dangling id. */
export function memberById(list: Member[], id: string | undefined | null): Member | undefined {
  if (!id) return undefined;
  return list.find((m) => m.id === id);
}
