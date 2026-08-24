/* MemberPicker.tsx — a compact, touch-first picker over the registered
 * members, plus a "no one" choice.
 *
 * Shared by the hub's two set-flows that assign a person: Menu's
 * plan-a-meal form (the cook) and Today's task reassign action. Kept as one
 * component so the picker's UX and styling live in a single place rather
 * than being duplicated across the two forms.
 *
 * Deliberately minimal (Task 9 Slice 2 scope): no search, no avatars, no
 * management affordances — just a list of touch targets to tap.
 */

import { For, Show } from "solid-js";
import type { Member } from "./api";

export default function MemberPicker(props: {
  members: Member[];
  /** The currently selected member id, or null for "no one". */
  selected: string | null;
  onSelect: (id: string | null) => void;
  disabled?: boolean;
}) {
  return (
    <div class="memberpicker" role="group" aria-label="Assign to">
      <button
        type="button"
        class="memberpicker-option"
        classList={{ "is-selected": props.selected === null }}
        disabled={props.disabled}
        onClick={() => props.onSelect(null)}
      >
        no one
      </button>
      <For each={props.members}>
        {(member) => (
          <button
            type="button"
            class="memberpicker-option"
            classList={{ "is-selected": props.selected === member.id }}
            disabled={props.disabled}
            onClick={() => props.onSelect(member.id)}
          >
            {/* Colour is a secondary scanning aid, never the sole signal —
                the name itself is always present (brief §12.4). Members
                without a colour just get the plain name. */}
            <Show when={member.color}>
              <span class={`member-dot member-dot-${member.color}`} aria-hidden="true" />
            </Show>
            {member.display_name}
          </button>
        )}
      </For>
    </div>
  );
}
