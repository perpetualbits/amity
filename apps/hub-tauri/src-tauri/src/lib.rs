// lib.rs — Tauri application library.
//
// Defines the Tauri commands exposed to the web frontend and sets up the
// Tauri application builder.
//
// Commands (each delegates to amity-service over HTTP on localhost):
//   capture_inbox_item  — POST /api/v1/inbox, returns the created item as JSON.
//   list_recent_inbox   — GET /api/v1/inbox/recent?limit=N, returns array of items.
//   surfacing_today     — GET /api/v1/surfacing/today?date=…, the Today view.
//   create_task         — POST /api/v1/tasks, from the capture form.
//   complete_task       — POST /api/v1/tasks/{id}/complete, mark an instance done.
//   change_assignee     — POST /api/v1/tasks/{id}/assignee, one-tap reassignment.
//
// The inbox commands were written in Task 1; the surfacing/task commands in
// Task 3 for the Today view and its task-capture form. All share the same
// reqwest + serde shape and the two small get_json / post_ok helpers below.
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
pub async fn capture_inbox_item(raw_text: String) -> Result<InboxItem, String> {
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
pub async fn list_recent_inbox(limit: u32) -> Result<Vec<InboxItem>, String> {
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
pub async fn surfacing_today(date: Option<String>) -> Result<TodayResponse, String> {
    // Build the URL, appending the optional date query parameter.
    let mut url = format!("{SERVICE_BASE_URL}/api/v1/surfacing/today");
    if let Some(date) = date {
        url = format!("{url}?date={date}");
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
pub async fn create_task(
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
pub async fn complete_task(id: String, instance_date: String) -> Result<(), String> {
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
pub async fn change_assignee(id: String, member_id: Option<String>) -> Result<(), String> {
    // The body carries the new assignee id (or null to clear it).
    let body = AssigneeBody { member_id };
    // POST to the task's assignee sub-resource.
    post_ok(
        format!("{SERVICE_BASE_URL}/api/v1/tasks/{id}/assignee"),
        &body,
    )
    .await
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
            create_task,
            complete_task,
            change_assignee,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
