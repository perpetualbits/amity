/* api.ts — typed wrappers over the Tauri commands.
 *
 * Every call into the Rust side goes through here so the components stay free of
 * `invoke` string literals and the camelCase→snake_case argument mapping lives
 * in one place. Tauri converts the camelCase keys below to the snake_case
 * parameter names on the Rust commands.
 */

import { invoke } from "@tauri-apps/api/core";

// ─── Types (mirror the service / Tauri response shapes) ─────────────────────

/** An inbox item as returned by the capture commands. */
export interface InboxItem {
  id: string;
  raw_text: string;
  captured_by: string;
  captured_at: string;
  source: string;
  triage_state: string;
  triaged_to?: string;
}

/** One item on the Today or Week view. `at` is an RFC 3339 instant. */
export interface SurfacedItem {
  kind: string; // "task" | "event"
  source_id: string;
  title: string;
  at: string;
  overdue: boolean;
  all_day: boolean;
  priority?: number;
  current_assignee_id?: string;
  /** A household note from an override; absent when there is none. */
  annotation?: string;
  /** True when an override moved this instance to a new time. */
  rescheduled: boolean;
}

/** The Today response envelope. */
export interface TodayResponse {
  date: string;
  has_surfaced: boolean;
  items: SurfacedItem[];
}

/** One day's bucket in a `WeekResponse`. */
export interface WeekDay {
  /** The calendar date this bucket is for (YYYY-MM-DD). */
  date: string;
  /** Items placed on this day, already ordered by the service — do not re-sort. */
  items: SurfacedItem[];
}

/** The Week response envelope: exactly 7 days, Monday-first. */
export interface WeekResponse {
  /** The Monday this week starts on (YYYY-MM-DD). */
  start: string;
  /** Exactly 7 day buckets, `start` through `start + 6 days`, in order. */
  days: WeekDay[];
}

/** Input for creating a task. Optional fields are omitted from the request. */
export interface CreateTaskInput {
  title: string;
  notes?: string;
  dueBy?: string;
  recurrenceRrule?: string;
  recurrenceTimezone?: string;
  tags?: string[];
}

/** One freetext ingredient line on a meal. */
export interface IngredientLine {
  name: string;
  qty?: string;
}

/** A planned meal, as returned by the meal commands. */
export interface Meal {
  id: string;
  /** The meal's calendar date (YYYY-MM-DD). */
  date: string;
  /** "dinner" | "breakfast" | "lunch" | "other". */
  slot: string;
  name: string;
  /** UUID of the cook, if assigned. */
  cook?: string;
  ingredient_lines: IngredientLine[];
  notes?: string;
  created_at: string;
}

/** Input for planning a meal from the Menu view's plan-a-meal form. */
export interface CreateMealInput {
  name: string;
  /** YYYY-MM-DD. */
  date: string;
  slot?: string;
  /** UUID of the cook, if assigned. */
  cook?: string;
  ingredientLines?: IngredientLine[];
  notes?: string;
}

/** A grocery list. */
export interface GroceryList {
  id: string;
  name: string;
  created_at: string;
}

/** One item on a grocery list. */
export interface GroceryItem {
  id: string;
  list_id: string;
  name: string;
  qty?: string;
  /** Free-form category, for grouping in the UI. */
  category?: string;
  checked: boolean;
  /** "manual" | "from_meal". */
  source: string;
  /** UUID of the meal this item was generated from; absent for manual items. */
  source_meal_id?: string;
  created_at: string;
}

/** Input for manually adding a grocery item. */
export interface AddGroceryItemInput {
  name: string;
  qty?: string;
  category?: string;
}

/** Result of generating grocery additions from planned meals. */
export interface GenerateGroceriesResult {
  /** The resolved inclusive lower bound of the meal date range used (YYYY-MM-DD). */
  from: string;
  /** The resolved inclusive upper bound of the meal date range used (YYYY-MM-DD). */
  to: string;
  /** The newly-added items (may be empty). */
  added: GroceryItem[];
}

/** A pantry staple. */
export interface PantryItem {
  id: string;
  name: string;
  note?: string;
  created_at: string;
}

/** Input for recording a pantry staple. */
export interface AddPantryInput {
  name: string;
  note?: string;
}

// ─── Inbox ──────────────────────────────────────────────────────────────────

/** Capture a free-text inbox item; returns the created item. */
export function captureInboxItem(rawText: string): Promise<InboxItem> {
  return invoke<InboxItem>("capture_inbox_item", { rawText });
}

/** List the most recent inbox items, newest first. */
export function listRecentInbox(limit: number): Promise<InboxItem[]> {
  return invoke<InboxItem[]>("list_recent_inbox", { limit });
}

// ─── Surfacing / Tasks ──────────────────────────────────────────────────────

/** Fetch the Today view; `date` is optional (YYYY-MM-DD). */
export function surfacingToday(date?: string): Promise<TodayResponse> {
  return invoke<TodayResponse>("surfacing_today", { date: date ?? null });
}

/** Fetch the Week view; `start` is optional (any date inside the target week,
 * YYYY-MM-DD) — absent means "this week". */
export function week(start?: string): Promise<WeekResponse> {
  return invoke<WeekResponse>("week", { start: start ?? null });
}

/** Create a task from the capture form. */
export function createTask(input: CreateTaskInput): Promise<void> {
  return invoke<void>("create_task", { ...input });
}

/** Mark a task instance done. `instanceDate` is YYYY-MM-DD. */
export function completeTask(id: string, instanceDate: string): Promise<void> {
  return invoke<void>("complete_task", { id, instanceDate });
}

/** Change a task's current assignee (null clears it). */
export function changeAssignee(id: string, memberId: string | null): Promise<void> {
  return invoke<void>("change_assignee", { id, memberId });
}

// ─── Meals ──────────────────────────────────────────────────────────────────

/** List meals, optionally within a date range (YYYY-MM-DD, both or neither). */
export function listMeals(from?: string, to?: string): Promise<Meal[]> {
  return invoke<Meal[]>("list_meals", { from: from ?? null, to: to ?? null });
}

/** Plan a meal from the Menu view's plan-a-meal form. Returns the created meal. */
export function createMeal(input: CreateMealInput): Promise<Meal> {
  return invoke<Meal>("create_meal", {
    name: input.name,
    date: input.date,
    slot: input.slot ?? null,
    cook: input.cook ?? null,
    ingredientLines: input.ingredientLines ?? null,
    notes: input.notes ?? null,
  });
}

// ─── Groceries ──────────────────────────────────────────────────────────────

/** List every grocery list. */
export function listGroceryLists(): Promise<GroceryList[]> {
  return invoke<GroceryList[]>("list_grocery_lists");
}

/** Create a grocery list. Returns the created list. */
export function createGroceryList(name: string): Promise<GroceryList> {
  return invoke<GroceryList>("create_grocery_list", { name });
}

/** List a grocery list's items. */
export function listGroceryItems(listId: string): Promise<GroceryItem[]> {
  return invoke<GroceryItem[]>("list_grocery_items", { listId });
}

/** Manually add a grocery item. Returns the created item. */
export function addGroceryItem(listId: string, input: AddGroceryItemInput): Promise<GroceryItem> {
  return invoke<GroceryItem>("add_grocery_item", {
    listId,
    name: input.name,
    qty: input.qty ?? null,
    category: input.category ?? null,
  });
}

/** Toggle a grocery item's checked state — the one free-tap mutation on Groceries. */
export function checkGroceryItem(id: string, checked: boolean): Promise<void> {
  return invoke<void>("check_grocery_item", { id, checked });
}

/** Remove a grocery item. */
export function deleteGroceryItem(id: string): Promise<void> {
  return invoke<void>("delete_grocery_item", { id });
}

/** Generate grocery additions from planned meals in a date range (absent means
 * the service's own default: the current Monday-Sunday week). Returns only the
 * newly-added items. */
export function generateGroceries(
  listId: string,
  from?: string,
  to?: string,
): Promise<GenerateGroceriesResult> {
  return invoke<GenerateGroceriesResult>("generate_groceries", {
    listId,
    from: from ?? null,
    to: to ?? null,
  });
}

// ─── Pantry ─────────────────────────────────────────────────────────────────

/** List every pantry staple. */
export function listPantry(): Promise<PantryItem[]> {
  return invoke<PantryItem[]>("list_pantry");
}

/** Record a pantry staple. Returns the created item. */
export function addPantry(input: AddPantryInput): Promise<PantryItem> {
  return invoke<PantryItem>("add_pantry", { name: input.name, note: input.note ?? null });
}

/** Remove a pantry staple. */
export function deletePantry(id: string): Promise<void> {
  return invoke<void>("delete_pantry", { id });
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

/** The placeholder member UUID (migration 0001) — the only member for now. */
export const PLACEHOLDER_MEMBER = "00000000-0000-7000-8000-000000000001";

/**
 * Format an RFC 3339 instant for display: "10:42" when it falls today,
 * otherwise a short "25 Jul" date. Uses the Dutch day-first convention.
 */
export function formatWhen(iso: string): string {
  const date = new Date(iso);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  if (sameDay) {
    // 24-hour time, per the Dutch convention (brief §16).
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  }
  return date.toLocaleDateString([], { day: "numeric", month: "short" });
}
