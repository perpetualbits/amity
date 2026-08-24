// lib.rs — Tauri application library.
//
// Defines the Tauri commands exposed to the web frontend and sets up the
// Tauri application builder.
//
// Commands (each delegates to amity-service over HTTP on localhost):
//   capture_inbox_item  — POST /api/v1/inbox, returns the created item as JSON.
//   list_recent_inbox   — GET /api/v1/inbox/recent?limit=N, returns array of items.
//   surfacing_today     — GET /api/v1/surfacing/today?date=…, the Today view.
//   week                — GET /api/v1/week?start=…, the Week view.
//   create_task         — POST /api/v1/tasks, from the capture form.
//   complete_task       — POST /api/v1/tasks/{id}/complete, mark an instance done.
//   change_assignee     — POST /api/v1/tasks/{id}/assignee, one-tap reassignment.
//   list_meals          — GET /api/v1/meals?from=&to=, the Menu view's data.
//   create_meal         — POST /api/v1/meals, from the Menu view's plan-a-meal form.
//   list_grocery_lists  — GET /api/v1/grocery-lists.
//   create_grocery_list — POST /api/v1/grocery-lists.
//   list_grocery_items  — GET /api/v1/grocery-lists/{id}/items.
//   add_grocery_item    — POST /api/v1/grocery-lists/{id}/items, manual add.
//   check_grocery_item  — PATCH /api/v1/grocery-items/{id}, tap-to-check.
//   delete_grocery_item — DELETE /api/v1/grocery-items/{id}.
//   generate_groceries  — POST /api/v1/grocery-lists/{id}/generate?from=&to=.
//   clear_checked_groceries — POST /api/v1/grocery-lists/{id}/clear-checked,
//                           the manual "clear checked" action (Task 9 Slice 3).
//   list_pantry         — GET /api/v1/pantry.
//   add_pantry          — POST /api/v1/pantry.
//   delete_pantry       — DELETE /api/v1/pantry/{id}.
//   list_members         — GET /api/v1/members, the member registry the
//                           frontend resolves ids against.
//
// The inbox commands were written in Task 1; the surfacing/task commands in
// Task 3 for the Today view and its task-capture form; the meal/grocery/pantry
// commands in P2 Slice 4 for the Menu and Groceries views; list_members in
// Task 9 Slice 2 for client-side member name resolution. All share the same
// reqwest + serde shape and the small get_json / post_ok / post_json / patch_ok
// / delete_ok helpers below.
//
// The service address is hardcoded to http://127.0.0.1:7890 for the prototype;
// a later task will read it from the Tauri app config or a sidecar-managed port.

use serde::{Deserialize, Serialize};

// ─── Service address ──────────────────────────────────────────────────────────

/// Base URL of the amity-service instance this application communicates with.
///
/// Hardcoded for the prototype. A later task will read this from the Tauri
/// app config or from a sidecar-managed port.
const SERVICE_BASE_URL: &str = "http://127.0.0.1:7890";

// ─── Shared data types ────────────────────────────────────────────────────────

/// An inbox item as returned by the service API.
///
/// Mirrors `InboxItemResponse` in amity-service. Kept as a separate type so
/// the frontend-facing shape can evolve independently of the service type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub raw_text: String,
    pub captured_by: String,
    pub captured_at: String,
    pub source: String,
    pub triage_state: String,
    pub triaged_to: Option<String>,
}

/// Request body for capturing a new inbox item.
#[derive(Debug, Serialize)]
struct CaptureRequest {
    raw_text: String,
    source: String,
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Capture a new inbox item by forwarding the request to amity-service.
///
/// Called from the frontend when the user submits the capture form.
/// Returns the created `InboxItem` on success, or an error string that the
/// frontend can display.
///
/// # Errors
///
/// Returns a string error if the HTTP request fails or the service returns
/// a non-2xx status. The frontend displays this as a plain message; no
/// toast notifications, no animated error states — consistent with the calm
/// aesthetic.
#[tauri::command]
async fn capture_inbox_item(raw_text: String) -> Result<InboxItem, String> {
    let client = reqwest::Client::new();

    let body = CaptureRequest {
        raw_text,
        source: "touch".to_owned(),
    };

    let response = client
        .post(format!("{SERVICE_BASE_URL}/api/v1/inbox"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {body}"));
    }

    response
        .json::<InboxItem>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

/// Fetch the most recent inbox items from amity-service.
///
/// Called from the frontend on mount and after each successful capture.
/// Returns a Vec of `InboxItem`, newest first.
///
/// # Errors
///
/// Returns a string error if the HTTP request fails.
#[tauri::command]
async fn list_recent_inbox(limit: u32) -> Result<Vec<InboxItem>, String> {
    let client = reqwest::Client::new();

    // Cap at 100 to match the service's own maximum, even though the service
    // would reject a higher limit itself. Belt-and-suspenders.
    let effective_limit = limit.min(100);

    let response = client
        .get(format!(
            "{SERVICE_BASE_URL}/api/v1/inbox/recent?limit={effective_limit}"
        ))
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(format!("service error {status}"));
    }

    response
        .json::<Vec<InboxItem>>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

// ─── Surfacing / Task types ─────────────────────────────────────────────────

/// One item on the Today view, mirroring `SurfacedItemResponse` in amity-service.
///
/// `at` is an RFC 3339 instant (the scheduled or due time); `overdue` is shown
/// as plain information, never a lateness count. Optional fields are absent when
/// the service omits them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfacedItem {
    /// Source entity type — `"task"` or `"event"`.
    pub kind: String,
    /// UUID of the source task or event (used for the complete / reassign
    /// actions on tasks, and client navigation).
    pub source_id: String,
    /// One-line title, shown verbatim.
    pub title: String,
    /// Salient instant (RFC 3339): scheduled time, or due/earliest time.
    pub at: String,
    /// True when the item is open past its deadline (tasks only; events never).
    pub overdue: bool,
    /// True for an all-day event; the hub shows "all day" instead of a time.
    pub all_day: bool,
    /// Importance rank 1-5; absent when not ranked.
    pub priority: Option<u8>,
    /// UUID of the member shown as responsible; absent when unset.
    pub current_assignee_id: Option<String>,
    /// A household note from an `Annotate` override; absent when there is none.
    pub annotation: Option<String>,
    /// True when a `Reschedule` override moved this instance to a new time.
    pub rescheduled: bool,
}

/// The Today response envelope, mirroring `TodayResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayResponse {
    /// The day this response is for (`YYYY-MM-DD`).
    pub date: String,
    /// Whether anything crossed the "surface today" threshold.
    pub has_surfaced: bool,
    /// The surfaced items, already ordered by the service.
    pub items: Vec<SurfacedItem>,
}

/// One day's bucket in a `WeekResponse`, mirroring `WeekDayResponse` in
/// amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeekDay {
    /// The calendar date this bucket is for (`YYYY-MM-DD`).
    pub date: String,
    /// The items placed on this day, already ordered by the service's layout
    /// rule (all-day first, then events before tasks, then by salient time).
    pub items: Vec<SurfacedItem>,
}

/// The Week response envelope, mirroring `WeekResponse` in amity-service.
///
/// Always exactly 7 `days`, Monday-first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeekResponse {
    /// The Monday this week starts on (`YYYY-MM-DD`).
    pub start: String,
    /// Exactly 7 day buckets, `start` through `start + 6 days`, in order.
    pub days: Vec<WeekDay>,
}

/// Request body for creating a task; unset fields are omitted from the JSON.
#[derive(Debug, Serialize)]
struct CreateTaskBody {
    /// One-line title; required.
    title: String,
    /// Optional free-form notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    /// Optional RFC 3339 deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    due_by: Option<String>,
    /// Optional RRULE string (paired with the timezone below).
    #[serde(skip_serializing_if = "Option::is_none")]
    recurrence_rrule: Option<String>,
    /// Optional IANA timezone; required when an RRULE is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    recurrence_timezone: Option<String>,
    /// Optional tags; normalised by the service.
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
}

/// Request body for completing a task instance.
#[derive(Debug, Serialize)]
struct CompleteBody {
    /// The scheduled date of the instance being resolved (`YYYY-MM-DD`).
    instance_date: String,
}

/// Request body for changing the current assignee (`null` clears it).
#[derive(Debug, Serialize)]
struct AssigneeBody {
    /// UUID of the member to set as current assignee, or `null`.
    member_id: Option<String>,
}

// ─── Meal / Grocery / Pantry types ──────────────────────────────────────────

/// One ingredient line, mirroring `IngredientLineResponse` in amity-service.
///
/// Used both ways: sent as part of `CreateMealBody` when planning a meal, and
/// received as part of `Meal` when listing them — the shape is identical on
/// the wire in both directions, so one struct covers both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngredientLine {
    /// The ingredient's name (e.g. "flour"), shown verbatim.
    pub name: String,
    /// Freetext quantity; omitted from the body when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
}

/// A planned meal, mirroring `MealResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meal {
    pub id: String,
    /// The meal's calendar date (`YYYY-MM-DD`).
    pub date: String,
    /// `"dinner"` | `"breakfast"` | `"lunch"` | `"other"`.
    pub slot: String,
    pub name: String,
    /// UUID of the cook, if assigned; absent when unset.
    pub cook: Option<String>,
    pub ingredient_lines: Vec<IngredientLine>,
    /// Free-form notes; absent when unset.
    pub notes: Option<String>,
    pub created_at: String,
}

/// Request body for `POST /api/v1/meals`; unset fields are omitted from the
/// JSON.
#[derive(Debug, Serialize)]
struct CreateMealBody {
    name: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cook: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingredient_lines: Option<Vec<IngredientLine>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

/// A grocery list, mirroring `GroceryListResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroceryList {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Request body for `POST /api/v1/grocery-lists`.
#[derive(Debug, Serialize)]
struct CreateGroceryListBody {
    name: String,
}

/// One grocery item, mirroring `GroceryItemResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroceryItem {
    pub id: String,
    pub list_id: String,
    pub name: String,
    /// Freetext quantity; absent when unset.
    pub qty: Option<String>,
    /// Free-form category, for grouping in the UI; absent when unset.
    pub category: Option<String>,
    pub checked: bool,
    /// `"manual"` or `"from_meal"`.
    pub source: String,
    /// UUID of the meal this item was generated from; absent for manual
    /// items.
    pub source_meal_id: Option<String>,
    pub created_at: String,
}

/// Request body for `POST /api/v1/grocery-lists/{id}/items` — a manual add.
#[derive(Debug, Serialize)]
struct CreateGroceryItemBody {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
}

/// Request body for `PATCH /api/v1/grocery-items/{id}`.
#[derive(Debug, Serialize)]
struct PatchGroceryItemBody {
    checked: bool,
}

/// Response envelope for `POST /api/v1/grocery-lists/{id}/generate`, mirroring
/// `GenerateResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateGroceriesResult {
    /// The resolved inclusive lower bound of the meal date range used
    /// (`YYYY-MM-DD`).
    pub from: String,
    /// The resolved inclusive upper bound of the meal date range used
    /// (`YYYY-MM-DD`).
    pub to: String,
    /// The newly-added items (may be empty).
    pub added: Vec<GroceryItem>,
}

/// Response envelope for `POST /api/v1/grocery-lists/{id}/clear-checked`,
/// mirroring the `{ "removed": N }` body the endpoint returns (see
/// amity-service's api/grocery.rs "clear-checked endpoint's contract" doc
/// comment). Kept as its own tiny struct — not reused for anything else — so
/// a future response-shape change there does not ripple into unrelated code.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClearCheckedResponse {
    /// How many checked items were deleted (0 when nothing was checked).
    removed: u64,
}

/// A pantry staple, mirroring `PantryItemResponse` in amity-service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PantryItem {
    pub id: String,
    pub name: String,
    /// Free-form note; absent when unset.
    pub note: Option<String>,
    pub created_at: String,
}

/// Request body for `POST /api/v1/pantry`.
#[derive(Debug, Serialize)]
struct CreatePantryItemBody {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

// ─── Member types ───────────────────────────────────────────────────────────

/// A household member, mirroring `MemberResponse` in amity-service.
///
/// The frontend fetches the whole roster once and resolves person ids (task
/// assignees, meal cooks) against it locally — see api.ts's shared members
/// store. A dangling id (no matching row, e.g. old seed data) is rendered as
/// "—" by the frontend, never treated as an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub display_name: String,
    /// Optional short label; absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    /// Optional accent colour (`snake_case` string, e.g. `"sage"`); absent
    /// when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub created_at: String,
}

// ─── HTTP helpers ───────────────────────────────────────────────────────────

/// GET `url` and deserialise the JSON body into `T`.
///
/// # Errors
///
/// Returns a message if the request fails, the status is non-2xx, or the body
/// cannot be parsed. The frontend displays it as plain text.
async fn get_json<T: serde::de::DeserializeOwned>(url: String) -> Result<T, String> {
    // One-shot client; the request volume here is a handful per interaction.
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    // Surface a non-success status rather than trying to parse an error body.
    if !response.status().is_success() {
        return Err(format!("service error {}", response.status()));
    }
    // Parse the JSON body into the caller's expected type.
    response
        .json::<T>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

/// POST `body` as JSON to `url`, discarding the response body on success.
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status; the service's
/// error text is included so the frontend can show the reason (e.g. a 422).
async fn post_ok<B: Serialize>(url: String, body: &B) -> Result<(), String> {
    // Send the JSON body to the service.
    let response = reqwest::Client::new()
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    // On a non-success status, include the service's message for context.
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {text}"));
    }
    // Success: the caller only needs to know it worked and will re-fetch.
    Ok(())
}

/// POST `body` as JSON to `url`, deserialising the response body into `T`.
///
/// Like `post_ok`, but for endpoints that return the created resource (e.g.
/// `POST /api/v1/meals` returns the created `Meal`) — the frontend can then
/// display it without a follow-up GET.
///
/// # Errors
///
/// Returns a message on transport failure, a non-2xx status, or a body that
/// does not parse as `T`.
async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
    url: String,
    body: &B,
) -> Result<T, String> {
    let response = reqwest::Client::new()
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {text}"));
    }
    response
        .json::<T>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

/// POST to `url` with no request body, deserialising the response into `T`.
///
/// Used for `POST /api/v1/grocery-lists/{id}/generate`, whose handler reads
/// its `from`/`to` range from query parameters rather than a JSON body.
///
/// # Errors
///
/// Returns a message on transport failure, a non-2xx status, or a body that
/// does not parse as `T`.
async fn post_no_body<T: serde::de::DeserializeOwned>(url: String) -> Result<T, String> {
    let response = reqwest::Client::new()
        .post(url)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {text}"));
    }
    response
        .json::<T>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

/// PATCH `body` as JSON to `url`, discarding the response body on success.
///
/// Mirrors `post_ok` but for PATCH — used for `PATCH /api/v1/grocery-items/{id}`
/// (tap-to-check), whose handler returns only a small confirmation body the
/// frontend does not need (it already knows the new state; it re-fetches the
/// list to refresh).
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
async fn patch_ok<B: Serialize>(url: String, body: &B) -> Result<(), String> {
    let response = reqwest::Client::new()
        .patch(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {text}"));
    }
    Ok(())
}

/// DELETE `url`, discarding the response body on success.
///
/// Used for the grocery-item and pantry-item removal commands, whose handlers
/// return only a small confirmation body the frontend does not need — it
/// re-fetches the list to refresh.
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
async fn delete_ok(url: String) -> Result<(), String> {
    let response = reqwest::Client::new()
        .delete(url)
        .send()
        .await
        .map_err(|e| format!("failed to reach amity-service: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("service error {status}: {text}"));
    }
    Ok(())
}

// ─── Surfacing / Task commands ──────────────────────────────────────────────

/// Fetch the Today view (`GET /api/v1/surfacing/today`), optionally for a date.
///
/// Called on mount of the Today view. `date` is `YYYY-MM-DD`; when absent the
/// service uses its own current date.
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn surfacing_today(date: Option<String>) -> Result<TodayResponse, String> {
    // Build the URL, appending the optional date query parameter.
    let mut url = format!("{SERVICE_BASE_URL}/api/v1/surfacing/today");
    if let Some(date) = date {
        url = format!("{url}?date={date}");
    }
    // Delegate to the shared GET helper.
    get_json(url).await
}

/// Fetch the Week view (`GET /api/v1/week`), optionally anchored to a date.
///
/// Called on mount of the Week view and on prev/next navigation. `start` is
/// any date inside the target week (`YYYY-MM-DD`); when absent the service
/// resolves "this week" from its own current date.
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn week(start: Option<String>) -> Result<WeekResponse, String> {
    // Build the URL, appending the optional start query parameter.
    let mut url = format!("{SERVICE_BASE_URL}/api/v1/week");
    if let Some(start) = start {
        url = format!("{url}?start={start}");
    }
    // Delegate to the shared GET helper.
    get_json(url).await
}

/// Create a task (`POST /api/v1/tasks`) from the capture form.
///
/// The recurrence pair (rrule + timezone) is either both present or both absent;
/// the frontend enforces that before calling. Returns unit on success — the
/// caller re-fetches the Today view.
///
/// # Errors
///
/// Returns the service's 422 message for invalid input (e.g. an empty title).
#[tauri::command]
async fn create_task(
    title: String,
    notes: Option<String>,
    due_by: Option<String>,
    recurrence_rrule: Option<String>,
    recurrence_timezone: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<(), String> {
    // Assemble the request body; None fields are omitted from the JSON.
    let body = CreateTaskBody {
        title,
        notes,
        due_by,
        recurrence_rrule,
        recurrence_timezone,
        tags,
    };
    // POST it and report success or the service error.
    post_ok(format!("{SERVICE_BASE_URL}/api/v1/tasks"), &body).await
}

/// Mark a task instance done (`POST /api/v1/tasks/{id}/complete`).
///
/// `instance_date` is the scheduled/due date (`YYYY-MM-DD`) of the surfaced
/// item; the frontend derives it from the item's `at` timestamp.
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
#[tauri::command]
async fn complete_task(id: String, instance_date: String) -> Result<(), String> {
    // The completion body only needs the instance date.
    let body = CompleteBody { instance_date };
    // POST to the task's complete sub-resource.
    post_ok(
        format!("{SERVICE_BASE_URL}/api/v1/tasks/{id}/complete"),
        &body,
    )
    .await
}

/// Change a task's current assignee (`POST /api/v1/tasks/{id}/assignee`).
///
/// The one-tap reassignment affordance. With a single placeholder member this
/// is mostly scaffolding, but the call is real end-to-end.
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
#[tauri::command]
async fn change_assignee(id: String, member_id: Option<String>) -> Result<(), String> {
    // The body carries the new assignee id (or null to clear it).
    let body = AssigneeBody { member_id };
    // POST to the task's assignee sub-resource.
    post_ok(
        format!("{SERVICE_BASE_URL}/api/v1/tasks/{id}/assignee"),
        &body,
    )
    .await
}

// ─── Meal commands ──────────────────────────────────────────────────────────

/// Fetch meals (`GET /api/v1/meals`), optionally within a date range.
///
/// Called on mount of the Menu view and on prev/next week navigation. `from`
/// and `to` (`YYYY-MM-DD`) must both be present or both absent, matching the
/// service's own pairing rule; the frontend enforces that before calling.
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn list_meals(from: Option<String>, to: Option<String>) -> Result<Vec<Meal>, String> {
    let mut url = format!("{SERVICE_BASE_URL}/api/v1/meals");
    if let (Some(from), Some(to)) = (from, to) {
        url = format!("{url}?from={from}&to={to}");
    }
    get_json(url).await
}

/// Plan a meal (`POST /api/v1/meals`) from the Menu view's plan-a-meal form.
///
/// Returns the created `Meal` so the caller can display it without a
/// follow-up GET.
///
/// # Errors
///
/// Returns the service's 422 message for invalid input (e.g. a blank name).
#[tauri::command]
async fn create_meal(
    name: String,
    date: String,
    slot: Option<String>,
    cook: Option<String>,
    ingredient_lines: Option<Vec<IngredientLine>>,
    notes: Option<String>,
) -> Result<Meal, String> {
    let body = CreateMealBody {
        name,
        date,
        slot,
        cook,
        ingredient_lines,
        notes,
    };
    post_json(format!("{SERVICE_BASE_URL}/api/v1/meals"), &body).await
}

// ─── Grocery commands ───────────────────────────────────────────────────────

/// List every grocery list (`GET /api/v1/grocery-lists`).
///
/// Called on mount of the Groceries view to resolve the active list.
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn list_grocery_lists() -> Result<Vec<GroceryList>, String> {
    get_json(format!("{SERVICE_BASE_URL}/api/v1/grocery-lists")).await
}

/// Create a grocery list (`POST /api/v1/grocery-lists`).
///
/// Called once, the first time the Groceries view finds no existing list.
///
/// # Errors
///
/// Returns the service's 422 message for a blank name.
#[tauri::command]
async fn create_grocery_list(name: String) -> Result<GroceryList, String> {
    let body = CreateGroceryListBody { name };
    post_json(format!("{SERVICE_BASE_URL}/api/v1/grocery-lists"), &body).await
}

/// List a grocery list's items (`GET /api/v1/grocery-lists/{id}/items`).
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable
/// or the list id is unknown (404).
#[tauri::command]
async fn list_grocery_items(list_id: String) -> Result<Vec<GroceryItem>, String> {
    get_json(format!(
        "{SERVICE_BASE_URL}/api/v1/grocery-lists/{list_id}/items"
    ))
    .await
}

/// Manually add a grocery item (`POST /api/v1/grocery-lists/{id}/items`).
///
/// Returns the created `GroceryItem` so the caller can display it without a
/// follow-up GET.
///
/// # Errors
///
/// Returns the service's 422 message for a blank name.
#[tauri::command]
async fn add_grocery_item(
    list_id: String,
    name: String,
    qty: Option<String>,
    category: Option<String>,
) -> Result<GroceryItem, String> {
    let body = CreateGroceryItemBody {
        name,
        qty,
        category,
    };
    post_json(
        format!("{SERVICE_BASE_URL}/api/v1/grocery-lists/{list_id}/items"),
        &body,
    )
    .await
}

/// Toggle a grocery item's checked state (`PATCH /api/v1/grocery-items/{id}`).
///
/// The hub's one free-tap mutation on the Groceries view — tap an item to
/// check it off. The caller re-fetches the list to refresh.
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
#[tauri::command]
async fn check_grocery_item(id: String, checked: bool) -> Result<(), String> {
    let body = PatchGroceryItemBody { checked };
    patch_ok(
        format!("{SERVICE_BASE_URL}/api/v1/grocery-items/{id}"),
        &body,
    )
    .await
}

/// Remove a grocery item (`DELETE /api/v1/grocery-items/{id}`).
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
#[tauri::command]
async fn delete_grocery_item(id: String) -> Result<(), String> {
    delete_ok(format!("{SERVICE_BASE_URL}/api/v1/grocery-items/{id}")).await
}

/// Generate grocery additions from planned meals
/// (`POST /api/v1/grocery-lists/{id}/generate`).
///
/// `from`/`to` (`YYYY-MM-DD`) are optional — absent means the service's own
/// default (the current Monday-Sunday week). Returns only the newly-added
/// items; the caller re-fetches the list to see the full picture.
///
/// # Errors
///
/// Returns a message on transport failure, a non-2xx status, or an unparsable
/// response.
#[tauri::command]
async fn generate_groceries(
    list_id: String,
    from: Option<String>,
    to: Option<String>,
) -> Result<GenerateGroceriesResult, String> {
    let mut url = format!("{SERVICE_BASE_URL}/api/v1/grocery-lists/{list_id}/generate");
    if let (Some(from), Some(to)) = (from, to) {
        url = format!("{url}?from={from}&to={to}");
    }
    post_no_body(url).await
}

/// Bulk-remove every checked item on a list
/// (`POST /api/v1/grocery-lists/{id}/clear-checked`) — the manual "clear
/// checked" action (Task 9 Slice 3). See amity-service's api/grocery.rs
/// "clear-checked endpoint's contract" doc comment for why this exists: a
/// checked (bought) item left on the list would otherwise block its own
/// re-addition by a later `generate_groceries` call. Not `pub` and not
/// wired to any timer — this only ever runs when the frontend's two-tap
/// confirm completes, matching the brief's "manual only".
///
/// Returns the number of items removed.
///
/// # Errors
///
/// Returns a message on transport failure, a non-2xx status, or an unparsable
/// response.
#[tauri::command]
async fn clear_checked_groceries(list_id: String) -> Result<u64, String> {
    let url = format!("{SERVICE_BASE_URL}/api/v1/grocery-lists/{list_id}/clear-checked");
    let resp: ClearCheckedResponse = post_no_body(url).await?;
    Ok(resp.removed)
}

// ─── Pantry commands ────────────────────────────────────────────────────────

/// List every pantry staple (`GET /api/v1/pantry`).
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn list_pantry() -> Result<Vec<PantryItem>, String> {
    get_json(format!("{SERVICE_BASE_URL}/api/v1/pantry")).await
}

/// Record a pantry staple (`POST /api/v1/pantry`).
///
/// Returns the created `PantryItem` so the caller can display it without a
/// follow-up GET.
///
/// # Errors
///
/// Returns the service's 422 message for a blank name.
#[tauri::command]
async fn add_pantry(name: String, note: Option<String>) -> Result<PantryItem, String> {
    let body = CreatePantryItemBody { name, note };
    post_json(format!("{SERVICE_BASE_URL}/api/v1/pantry"), &body).await
}

/// Remove a pantry staple (`DELETE /api/v1/pantry/{id}`).
///
/// # Errors
///
/// Returns a message on transport failure or a non-2xx status.
#[tauri::command]
async fn delete_pantry(id: String) -> Result<(), String> {
    delete_ok(format!("{SERVICE_BASE_URL}/api/v1/pantry/{id}")).await
}

// ─── Member commands ────────────────────────────────────────────────────────

/// List every registered member (`GET /api/v1/members`).
///
/// Called once by the frontend's shared members resource (see api.ts /
/// members.ts); the hub resolves person ids (task assignees, meal cooks)
/// against this list locally rather than the service inlining names.
///
/// # Errors
///
/// Returns a message the frontend can display if the service is unreachable.
#[tauri::command]
async fn list_members() -> Result<Vec<Member>, String> {
    // The service wraps the list in a `{ "members": [...] }` envelope
    // (matching list_calendars_handler); unwrap it here so the frontend gets
    // a bare array, like list_pantry.
    #[derive(Deserialize)]
    struct MembersEnvelope {
        members: Vec<Member>,
    }
    let envelope: MembersEnvelope = get_json(format!("{SERVICE_BASE_URL}/api/v1/members")).await?;
    Ok(envelope.members)
}

// ─── Application entry ────────────────────────────────────────────────────────

/// Build and run the Tauri application.
///
/// Called from `main.rs`. Registers all Tauri commands so the frontend can
/// invoke them via `invoke(...)`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            capture_inbox_item,
            list_recent_inbox,
            surfacing_today,
            week,
            create_task,
            complete_task,
            change_assignee,
            list_meals,
            create_meal,
            list_grocery_lists,
            create_grocery_list,
            list_grocery_items,
            add_grocery_item,
            check_grocery_item,
            delete_grocery_item,
            generate_groceries,
            clear_checked_groceries,
            list_pantry,
            add_pantry,
            delete_pantry,
            list_members,
        ])
        // Kiosk mode: when AMITY_KIOSK=1 (set by scripts/run-hub.sh), start the
        // window fullscreen for the wall-mounted hub. Default (unset/other) is a
        // normal window. A missing window or a set-fullscreen failure is ignored
        // — kiosk is a display preference, never a reason to fail startup.
        .setup(|app| {
            use tauri::Manager;
            if std::env::var("AMITY_KIOSK").is_ok_and(|v| v == "1") {
                for (_label, window) in app.webview_windows() {
                    let _ = window.set_fullscreen(true);
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
