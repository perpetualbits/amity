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

/** One item on the Today view. `at` is an RFC 3339 instant. */
export interface SurfacedItem {
  kind: string;
  source_id: string;
  title: string;
  status: string;
  at: string;
  overdue: boolean;
  priority?: number;
  current_assignee_id?: string;
}

/** The Today response envelope. */
export interface TodayResponse {
  date: string;
  has_surfaced: boolean;
  items: SurfacedItem[];
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
