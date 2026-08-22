/* Groceries.tsx — the current grocery list.
 *
 * Resolves (or creates) the household's single active grocery list, then
 * shows its items: name, quantity, and category grouping when items carry
 * one. Tap-to-check is the one free mutation here (brief) — tapping an
 * item's row toggles it checked, no confirmation, no toast (matching
 * Capture's quiet feedback). A minimal manual-add flow (name + optional qty)
 * and a delete affordance round out manual editing; an explicit "Generate
 * from this week's menu" button turns planned meals into new items without
 * ever touching what is already on the list (see amity-service's
 * api/grocery.rs generate_grocery_items doc comment for the no-clobber
 * contract this relies on).
 */

import { createSignal, onMount, For, Show } from "solid-js";
import {
  addGroceryItem,
  checkGroceryItem,
  createGroceryList,
  deleteGroceryItem,
  generateGroceries,
  listGroceryItems,
  listGroceryLists,
  type GroceryItem,
} from "./api";

/** The single grocery list's display name — created once if none exists yet. */
const DEFAULT_LIST_NAME = "Groceries";

export default function Groceries() {
  // The active list's id; null until resolved (or created) on mount.
  const [listId, setListId] = createSignal<string | null>(null);
  // The active list's items.
  const [items, setItems] = createSignal<GroceryItem[]>([]);
  // True until the list is resolved and its items load, so we do not flash
  // an empty list.
  const [loading, setLoading] = createSignal(true);
  // Error message when a fetch or mutation fails; null when there is none.
  const [error, setError] = createSignal<string | null>(null);
  // True while a mutation (check/delete/add/generate) is in flight.
  const [busy, setBusy] = createSignal(false);
  // Whether the manual-add form is open.
  const [addOpen, setAddOpen] = createSignal(false);

  /** Resolve the household's single active grocery list: use the first
   * existing one, or create "Groceries" if none exists yet. */
  async function resolveList(): Promise<string> {
    const lists = await listGroceryLists();
    if (lists.length > 0) return lists[0].id;
    const created = await createGroceryList(DEFAULT_LIST_NAME);
    return created.id;
  }

  /** Load the active list's items, resolving the list first if needed. */
  async function load() {
    setError(null);
    try {
      const id = listId() ?? (await resolveList());
      setListId(id);
      setItems(await listGroceryItems(id));
    } catch (err) {
      setError(typeof err === "string" ? err : "could not load groceries");
    } finally {
      setLoading(false);
    }
  }

  onMount(load);

  /** Tap-to-check: toggle an item's checked state, then reload. */
  async function toggleChecked(item: GroceryItem) {
    setBusy(true);
    try {
      await checkGroceryItem(item.id, !item.checked);
      await load();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not update item");
    } finally {
      setBusy(false);
    }
  }

  /** Delete an item, then reload. */
  async function removeItem(item: GroceryItem) {
    setBusy(true);
    try {
      await deleteGroceryItem(item.id);
      await load();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not remove item");
    } finally {
      setBusy(false);
    }
  }

  /** Generate additions from this week's planned meals, then reload. The
   * service defaults `from`/`to` to the current Monday-Sunday week when
   * absent, so no client-side date math is needed here. */
  async function generateFromMenu() {
    const id = listId();
    if (!id) return;
    setBusy(true);
    try {
      await generateGroceries(id);
      await load();
    } catch (err) {
      setError(typeof err === "string" ? err : "could not generate from menu");
    } finally {
      setBusy(false);
    }
  }

  /** Group items by category, in first-seen order; uncategorised items form
   * their own (unlabelled) leading group. Degrades to a single flat group
   * when no item carries a category — the common case today, since neither
   * manual add nor generation sets one yet. */
  const groups = () => {
    const uncategorised: GroceryItem[] = [];
    const byCategory = new Map<string, GroceryItem[]>();
    for (const item of items()) {
      if (item.category) {
        const bucket = byCategory.get(item.category) ?? [];
        bucket.push(item);
        byCategory.set(item.category, bucket);
      } else {
        uncategorised.push(item);
      }
    }
    const result: { label: string | null; items: GroceryItem[] }[] = [];
    if (uncategorised.length > 0) result.push({ label: null, items: uncategorised });
    for (const [label, groupItems] of byCategory) {
      result.push({ label, items: groupItems });
    }
    return result;
  };

  return (
    <>
      <section class="groceries-section" aria-label="Groceries">
        <div class="groceries-toolbar">
          <button
            class="groceries-generate"
            type="button"
            disabled={busy() || loading() || !listId()}
            onClick={generateFromMenu}
          >
            Generate from this week's menu
          </button>
        </div>

        <Show when={error()}>
          <p class="capture-error" role="alert">
            {error()}
          </p>
        </Show>

        <Show when={!loading()}>
          <Show when={items().length > 0} fallback={<p class="empty-state">nothing on the list</p>}>
            <For each={groups()}>
              {(group) => (
                <div class="grocery-group">
                  <Show when={group.label}>
                    <h3 class="grocery-group-label">{group.label}</h3>
                  </Show>
                  <ul class="grocery-list">
                    <For each={group.items}>
                      {(item) => (
                        <li class="grocery-item" classList={{ "is-checked": item.checked }}>
                          <button
                            class="grocery-item-tap"
                            type="button"
                            disabled={busy()}
                            aria-pressed={item.checked}
                            aria-label={`${item.checked ? "Uncheck" : "Check"} ${item.name}`}
                            onClick={() => toggleChecked(item)}
                          >
                            <span class="grocery-item-mark" aria-hidden="true">
                              {item.checked ? "☑" : "☐"}
                            </span>
                            <span class="grocery-item-name">{item.name}</span>
                            <Show when={item.qty}>
                              <span class="grocery-item-qty">{item.qty}</span>
                            </Show>
                          </button>
                          <button
                            class="grocery-item-delete"
                            type="button"
                            disabled={busy()}
                            aria-label={`Remove ${item.name}`}
                            onClick={() => removeItem(item)}
                          >
                            ✕
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </section>

      <section class="groceryadd-section" aria-label="Add a grocery item">
        <Show
          when={addOpen()}
          fallback={
            <button class="addtask-toggle" type="button" onClick={() => setAddOpen(true)}>
              + Add an item
            </button>
          }
        >
          <AddItemForm
            busy={busy()}
            setBusy={setBusy}
            onAdd={async (input) => {
              const id = listId();
              if (!id) return;
              await addGroceryItem(id, input);
            }}
            onCreated={async () => {
              setAddOpen(false);
              await load();
            }}
            onCancel={() => setAddOpen(false)}
            onError={setError}
          />
        </Show>
      </section>
    </>
  );
}

/** The minimal manual-add form: name and optional quantity (brief). */
function AddItemForm(props: {
  busy: boolean;
  setBusy: (b: boolean) => void;
  onAdd: (input: { name: string; qty?: string }) => Promise<void>;
  onCreated: () => void | Promise<void>;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const [name, setName] = createSignal("");
  const [qty, setQty] = createSignal("");

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const itemName = name().trim();
    if (!itemName) return;
    props.setBusy(true);
    try {
      await props.onAdd({ name: itemName, qty: qty().trim() || undefined });
      await props.onCreated();
    } catch (err) {
      props.onError(typeof err === "string" ? err : "could not add item");
    } finally {
      props.setBusy(false);
    }
  }

  return (
    <form class="itemadd-form" onSubmit={handleSubmit}>
      <input
        class="capture-input"
        type="text"
        value={name()}
        onInput={(e) => setName(e.currentTarget.value)}
        placeholder="item name"
        autocomplete="off"
        required
      />
      <input
        class="capture-input itemadd-qty"
        type="text"
        value={qty()}
        onInput={(e) => setQty(e.currentTarget.value)}
        placeholder="qty (optional)"
        autocomplete="off"
      />
      <div class="itemadd-actions">
        <button
          class="capture-submit itemadd-submit"
          type="submit"
          disabled={props.busy || !name().trim()}
        >
          Add
        </button>
        <button class="taskform-cancel" type="button" onClick={props.onCancel} disabled={props.busy}>
          Cancel
        </button>
      </div>
    </form>
  );
}
