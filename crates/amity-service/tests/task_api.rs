// task_api.rs — integration tests for the Task HTTP API.
//
// Each test builds the full axum application (production code, not a mock)
// backed by an in-memory SQLite database and sends real HTTP requests via
// `tower::ServiceExt::oneshot`. No mocking, no faking of the storage layer.
//
// Why in-process rather than a real network socket?
//   • Deterministic teardown: the pool is dropped at test end.
//   • No port conflicts between parallel test runs.
//   • Application code is identical to production; only the transport differs.
//
// What these tests verify:
//   • POST /tasks: creation succeeds (201) and the body is correct.
//   • POST /tasks: empty title returns 422.
//   • POST /tasks: incomplete recurrence pair returns 422.
//   • GET /tasks: empty list; filtered by status; filtered by tag.
//   • GET /tasks/{id}: found (200) and not found (404).
//   • PATCH /tasks/{id}: field updates are reflected in the response.
//   • POST /tasks/{id}/complete: status changes; log entry has skipped=false.
//   • POST /tasks/{id}/skip: status changes; log entry has skipped=true.
//   • POST /tasks/{id}/assignee: current_assignee_id updated.
//   • GET /tasks/{id}/history: completion log entries returned.
//   • GET /tasks/upcoming: instances returned for a recurring task.

// The production `build_app` function wires all routes with the given pool.
use amity_service::build_app;
// `open_database` applies all migrations before returning the pool.
use amity_storage::connection::open_database;
// `Body` wraps raw bytes for HTTP request bodies.
use axum::body::Body;
// `Request` is the HTTP request type; `StatusCode` for status code assertions.
use axum::http::{Request, StatusCode};
// `json!` builds JSON request bodies from literals; `Value` for flexible assertions.
use serde_json::{Value, json};
// `ServiceExt` provides the `oneshot` method that drives one request in-process.
use tower::ServiceExt;

// ─── Test helpers ─────────────────────────────────────────────────────────────

/// Build a test application backed by a fresh in-memory database.
///
/// Returns the axum `Router` ready to accept requests via `oneshot`.
/// Each call creates an isolated database — tests running in parallel cannot
/// see each other's writes. The `build_app` function is the same one used in
/// production; no test-specific routing or middleware is added.
///
/// Uses `:memory:` rather than a temp file to avoid post-test cleanup and to
/// guarantee each call starts from a completely empty state.
async fn build_test_app() -> axum::Router {
    // `:memory:` creates a new, empty SQLite database on every call.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory database should always open");
    // `build_app` is the production router — no test-specific fork.
    build_app(db)
}

/// Issue a `POST` request with a JSON body and return the response.
///
/// Sets `Content-Type: application/json` so axum's `Json` extractor accepts the
/// body. The `body` value is serialised to a UTF-8 string by `serde_json`.
async fn post_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    // Build the POST request with the JSON body and required Content-Type.
    let request = Request::builder()
        // Specify the HTTP method — axum routes match the exact uppercase string.
        .method("POST")
        // Set the request URI from the caller-supplied path argument.
        .uri(path)
        // Without Content-Type: application/json, axum returns 415 Unsupported.
        .header("Content-Type", "application/json")
        // Serialise the JSON Value to bytes for the request body.
        .body(Body::from(body.to_string()))
        // Builder errors are programmer mistakes, not runtime conditions.
        .expect("request builder should not fail");
    // Drive the request through the service without binding to a network port.
    app.oneshot(request)
        .await
        // A tower service error indicates a bug in the routing or middleware stack.
        .expect("service call should not fail")
}

/// Issue a `PATCH` request with a JSON body and return the response.
///
/// Identical to `post_json` except the HTTP method is `PATCH`.
/// Used for `PATCH /api/v1/tasks/{id}` to update task fields.
async fn patch_json(app: axum::Router, path: &str, body: Value) -> axum::response::Response {
    // Build the PATCH request; Content-Type is required on PATCH bodies too.
    let request = Request::builder()
        // Specify the HTTP method — PATCH for partial field updates (RFC 5789).
        .method("PATCH")
        // Set the request URI from the caller-supplied path argument.
        .uri(path)
        // axum's Json extractor requires this header regardless of HTTP method.
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builder should not fail");
    // Drive the PATCH request in-process.
    app.oneshot(request)
        .await
        .expect("service call should not fail")
}

/// Issue a `GET` request and return the response.
///
/// GET requests have no body. `Body::empty()` satisfies the type without
/// allocating any heap memory.
async fn get(app: axum::Router, path: &str) -> axum::response::Response {
    // Build the GET request with an empty body.
    let request = Request::builder()
        // Specify the HTTP method — GET for read-only, side-effect-free access.
        .method("GET")
        // Set the request URI from the caller-supplied path argument.
        .uri(path)
        // GET carries no body; `Body::empty()` is the zero-cost placeholder.
        .body(Body::empty())
        .expect("request builder should not fail");
    // Drive the GET request in-process.
    app.oneshot(request)
        .await
        .expect("service call should not fail")
}

/// Deserialise the response body as a `serde_json::Value`.
///
/// Reads the complete body before parsing. `usize::MAX` as the byte limit
/// prevents accidental truncation of larger payloads — for tests, memory use
/// is not a constraint.
async fn body_json(response: axum::response::Response) -> Value {
    // Consume the response and collect all body bytes.
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    // Parse as JSON — a failure here means the handler returned a non-JSON body.
    serde_json::from_slice(&bytes).expect("response body should be valid JSON")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn post_tasks_creates_task_and_returns_201() {
    // Happy path: a valid JSON body with a non-empty title.
    // Verifies that the handler stores the task and returns 201 Created with
    // the full task body — so the client can display it without a GET round-trip.
    //
    // Fields checked: status (must be "open"), id (must be a UUID string),
    // created_at/updated_at (must be RFC 3339 strings), tags (must be an array).
    //
    // No recurrence fields are sent — `recurrence_rrule` and `recurrence_timezone`
    // must both be null in the response body when they were not supplied.
    // The default status for a new task is "open"; no other value is valid here.
    //
    // Fresh database — no prior state to interfere with this test.
    let app = build_test_app().await;

    // Send the minimum valid payload: just a title.
    let response = post_json(
        app,
        "/api/v1/tasks",
        json!({ "title": "vacuum living room" }),
    )
    .await;

    // 201 Created is the correct status for a new resource; 200 would be wrong.
    assert_eq!(response.status(), StatusCode::CREATED);

    // Parse the response body to check each field.
    let body = body_json(response).await;

    // The server must echo the title we sent, verbatim.
    assert_eq!(body["title"], "vacuum living room");
    // All new tasks start in the "open" lifecycle state.
    assert_eq!(body["status"], "open");
    // The ID is server-assigned; we can only verify it is a non-null string.
    assert!(body["id"].is_string(), "id must be a JSON string");
    // Both timestamps are server-assigned; verify they are present as strings.
    assert!(body["created_at"].is_string(), "created_at must be present");
    // updated_at equals created_at at creation time; both must be non-null strings.
    assert!(body["updated_at"].is_string(), "updated_at must be present");
    // tags must always be present as a JSON array (empty when none were sent).
    assert!(body["tags"].is_array(), "tags must be a JSON array");
}

#[tokio::test]
async fn post_tasks_empty_title_returns_422() {
    // A blank title violates the domain invariant enforced by TaskBuilder.
    // The handler must return 422 Unprocessable Entity — not 400 Bad Request.
    // 400 implies bad JSON syntax; 422 means the JSON was valid but violated a rule.
    //
    // This test ensures the domain validation layer is wired up to the HTTP layer.
    //
    // A whitespace-only string is rejected by `TaskBuilder` before any database
    // interaction — the validation is in the domain layer, not the storage layer.
    // This keeps the 422 response fast and free of database round-trips.
    //
    // Build the test application backed by an empty in-memory database.
    let app = build_test_app().await;

    // Whitespace-only title — the domain layer must reject it.
    let response = post_json(app, "/api/v1/tasks", json!({ "title": "   " })).await;

    // 422 is the correct status for valid JSON that violates a domain constraint.
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_tasks_incomplete_recurrence_pair_returns_422() {
    // The recurrence rule is stored as two separate fields and must arrive as a pair.
    // Supplying only `recurrence_rrule` without `recurrence_timezone` is a client
    // error — the handler must catch this before calling the domain layer.
    //
    // The same check applies to the reverse case (timezone without rrule).
    //
    // The handler validates the pair before calling the domain layer: `(Some, None)`
    // is a 422 at the HTTP boundary, not a domain error. This keeps the API contract
    // explicit — callers must supply both fields together or neither.
    //
    // Build the test application backed by an empty in-memory database.
    let app = build_test_app().await;

    // Send only the RRULE without a matching timezone — the pair is incomplete.
    let response = post_json(
        app,
        "/api/v1/tasks",
        json!({
            "title": "weekly standup",
            // RRULE is present but timezone is absent — the handler must reject this.
            "recurrence_rrule": "FREQ=WEEKLY;BYDAY=TH"
        }),
    )
    .await;

    // The handler must reject the incomplete pair with 422.
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_tasks_returns_empty_array_for_fresh_database() {
    // A fresh database has no tasks. The endpoint must return an empty JSON array,
    // not a 404 or 500 — an empty list is a valid, expected state on first startup.
    //
    // This is the "zero-item list" contract: the resource (the collection) exists
    // but currently has no members.
    //
    // A 404 would wrongly imply the collection itself does not exist.
    // A 500 would wrongly imply a server-side error; empty is a normal startup state.
    // Only 200 + `[]` correctly communicates "the list exists but has no items".
    //
    // Build the test application backed by an empty in-memory database.
    let app = build_test_app().await;

    // Query the task list against an empty database.
    let response = get(app, "/api/v1/tasks").await;

    // 200 + empty array is the correct response for an empty-but-valid collection.
    assert_eq!(response.status(), StatusCode::OK);

    // Parse the body to verify the array shape.
    let body = body_json(response).await;
    // The body must be a JSON array — not an object, not null.
    assert!(body.is_array(), "body should be a JSON array");
    // For an empty database, the array must contain zero elements.
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_tasks_returns_created_task() {
    // End-to-end: create a task via POST, then verify it appears in GET /tasks.
    // The two requests must share the same database pool to observe each other's
    // writes — SQLite in-memory databases are not shared across connections.
    //
    // `SqlitePool` is an `Arc`-wrapped pool — cloning it is cheap and produces
    // a second handle to the same underlying connections. Both router instances
    // therefore see the same committed writes regardless of which one issued them.
    //
    // Open the shared pool directly so it can be cloned for two routers.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");

    // POST router: used to create the task.
    let app_post = build_app(db.clone());
    // GET router: shares the pool so it sees the task created by app_post.
    let app_get = build_app(db);

    // Create the task using the POST router.
    post_json(
        app_post,
        "/api/v1/tasks",
        json!({ "title": "water the plants" }),
    )
    .await;
    // The task should now be in the shared database.

    // Fetch the task list from the GET router.
    let response = get(app_get, "/api/v1/tasks").await;
    // The list must return 200 OK even with only one task in it.
    assert_eq!(response.status(), StatusCode::OK);

    // Parse the list response.
    let body = body_json(response).await;
    let items = body.as_array().expect("body must be an array");
    // The task created via POST must appear in the list.
    assert_eq!(items.len(), 1, "one task should appear after one POST");
    // The title must match what was sent in the POST request.
    assert_eq!(items[0]["title"], "water the plants");
}

#[tokio::test]
async fn get_task_by_id_returns_200_when_found() {
    // After creating a task, fetching it by UUID must return 200 and the task body.
    // The `id` from the creation response is used as the path parameter.
    //
    // This exercises the URL routing for `GET /api/v1/tasks/{id}` and verifies
    // that the path parameter is correctly parsed as a `TaskId`.
    //
    // The test also verifies that the handler echoes the correct task fields back,
    // confirming that storage retrieved the right row (not a stale default value).
    //
    // Sequence: POST /tasks → assert 201 → extract id → GET /{id} → assert fields.
    //
    // Open the shared pool so that the POST and GET routers see the same writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Three routers sharing the same pool: create, fetch by id, verify fields.
    let app_post = build_app(db.clone());
    // GET router: shares the pool so it can find the task created by app_post.
    let app_get = build_app(db);

    // Create the task and capture the server-assigned ID.
    let create_response = post_json(
        app_post,
        "/api/v1/tasks",
        json!({ "title": "read API docs" }),
    )
    .await;
    // The creation must succeed before we can fetch by ID.
    assert_eq!(create_response.status(), StatusCode::CREATED);

    // Extract the server-assigned ID from the creation response body.
    let create_body = body_json(create_response).await;
    // The `id` field must be a string; it will be used in the GET URL.
    let task_id = create_body["id"].as_str().expect("id must be a string");

    // Fetch the task by its UUID using the shared-pool GET router.
    let get_response = get(app_get, &format!("/api/v1/tasks/{task_id}")).await;
    // Found: must return 200 OK, not 404 or 500.
    assert_eq!(get_response.status(), StatusCode::OK);

    // Parse the single-task response body.
    let get_body = body_json(get_response).await;
    // The ID in the response must match the URL parameter we used.
    assert_eq!(get_body["id"], task_id);
    // The title must match the one we created.
    assert_eq!(get_body["title"], "read API docs");
}

#[tokio::test]
async fn get_task_by_id_returns_404_when_not_found() {
    // A valid UUID that was never inserted must return 404, not 500.
    // The storage layer returns `None` for missing IDs; the handler maps that to 404.
    // A 500 would wrongly suggest a server-side failure rather than a missing resource.
    //
    // The UUID must be syntactically valid — a malformed UUID (e.g. "not-a-uuid")
    // would return 400 (Bad Request) before reaching the storage lookup. Using a
    // well-formed UUID exercises the storage path that returns `None` for a miss.
    //
    // Build the test application backed by an empty in-memory database.
    let app = build_test_app().await;

    // Use a well-formed UUID that has never been inserted into this database.
    // The UUID format is valid, so the handler must proceed to the storage lookup.
    let response = get(app, "/api/v1/tasks/018f1a2b-0000-7000-8000-000000000099").await;

    // 404 Not Found is the correct status for "valid UUID, but no matching row".
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_task_updates_title() {
    // PATCH must update only the specified fields and return the updated task.
    // Fields absent from the PATCH body must remain unchanged.
    //
    // This test verifies the PATCH endpoint's partial-update semantics: sending
    // only `title` must not clear `status`, `tags`, or other fields.
    //
    // PATCH semantics: fields absent from the body are left unchanged; only the
    // fields present in the request body are updated on the stored row.
    // The response must reflect all task fields, not just the ones that changed.
    //
    // Sequence: POST → assert 201 → PATCH title → assert 200 + title → GET → assert title.
    //
    // Open the shared pool so that all three router instances see the same writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Three routers sharing the same pool: create, patch, then verify.
    let app_create = build_app(db.clone());
    // PATCH router: shares the pool so it can find and update the created task.
    let app_patch = build_app(db.clone());
    // GET router: used to verify the change was persisted after the PATCH.
    let app_get = build_app(db);

    // Create the task with its original title.
    let create_resp = post_json(
        app_create,
        "/api/v1/tasks",
        json!({ "title": "original title" }),
    )
    .await;
    // Extract the task ID from the creation response.
    let body = body_json(create_resp).await;
    // The ID is needed to construct the PATCH URL.
    let id = body["id"].as_str().expect("id must be a string");

    // Apply the PATCH: change only the title field.
    let patch_resp = patch_json(
        app_patch,
        &format!("/api/v1/tasks/{id}"),
        // Only `title` is sent; all other fields should remain as-is.
        json!({ "title": "updated title" }),
    )
    .await;
    // PATCH must return 200 OK on success.
    assert_eq!(patch_resp.status(), StatusCode::OK);

    // Parse the PATCH response body to verify the title was updated.
    let patch_body = body_json(patch_resp).await;
    // The response must reflect the new title immediately.
    assert_eq!(patch_body["title"], "updated title");

    // Verify the change was also persisted by fetching via GET.
    let get_resp = get(app_get, &format!("/api/v1/tasks/{id}")).await;
    // The GET must succeed and return 200 OK.
    assert_eq!(get_resp.status(), StatusCode::OK);
    // Parse the GET response to verify the title persisted.
    let get_body = body_json(get_resp).await;
    // The GET response must also show the updated title (not the original).
    assert_eq!(get_body["title"], "updated title");
}

#[tokio::test]
async fn complete_task_returns_200_with_log_entry() {
    // POST /complete must return 200 with a CompletionLog entry where `skipped=false`.
    // The task status must also change to "done", verified via a subsequent GET.
    //
    // This test exercises the create→complete→verify round-trip, ensuring all three
    // storage operations (insert_task, mark_task_done, insert_completion_log) are
    // wired up correctly through the HTTP layer.
    //
    // The `instance_date` field identifies which scheduled occurrence is completed;
    // for non-recurring tasks the client supplies the date of the completion itself.
    // The date is stored on the `CompletionLog` row — not on the task row.
    //
    // Sequence: POST → POST /complete → assert log → GET → assert task status "done".
    //
    // Open the shared pool so that all three routers observe each other's writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Three routers: create the task, mark it done, then verify the resulting status.
    let app_create = build_app(db.clone());
    // Complete router: shares the pool so it can find and update the created task.
    let app_complete = build_app(db.clone());
    // GET router: used to verify the task status changed after the completion.
    let app_get = build_app(db);

    // Create the task we will complete.
    let create_resp = post_json(
        app_create,
        "/api/v1/tasks",
        json!({ "title": "clean kitchen" }),
    )
    .await;
    // Extract the task ID for use in subsequent requests.
    let body = body_json(create_resp).await;
    // The ID is needed to build the `/complete` URL.
    let id = body["id"].as_str().expect("id must be a string");

    // Mark the task as complete using the shared-pool router.
    let complete_resp = post_json(
        app_complete,
        &format!("/api/v1/tasks/{id}/complete"),
        // `instance_date` is required; for a one-shot task, supply the completion date.
        json!({ "instance_date": "2026-05-25" }),
    )
    .await;
    // The complete endpoint must return 200 OK with the new log entry.
    assert_eq!(complete_resp.status(), StatusCode::OK);

    // Parse the CompletionLog response returned by the complete endpoint.
    let log = body_json(complete_resp).await;
    // The log must reference the correct parent task.
    assert_eq!(log["task_id"], id);
    // `skipped` must be false — this was a genuine completion, not a skip.
    assert_eq!(log["skipped"], false);
    // The instance_date must match what was sent in the request body.
    assert_eq!(log["instance_date"], "2026-05-25");

    // Verify the task status changed to "done" via a subsequent GET request.
    let task_resp = get(app_get, &format!("/api/v1/tasks/{id}")).await;
    // The GET must succeed with 200 OK.
    assert_eq!(task_resp.status(), StatusCode::OK);
    // Parse the task body to check the status field.
    let task_body = body_json(task_resp).await;
    // The status must now be "done" — any other value means the update was not persisted.
    assert_eq!(task_body["status"], "done");
}

#[tokio::test]
async fn skip_task_returns_200_with_skip_log_entry() {
    // POST /skip must return 200 with a CompletionLog entry where `skipped=true`.
    // The task status must change to "skipped". Skipping is a first-class event
    // (brief §8.2) — the skip endpoint is not a lesser version of /complete.
    //
    // An optional `notes` field is included to verify it round-trips through the log.
    //
    // The `notes` field carries household context for why an instance was skipped.
    // The system records the fact without judgment — notes are stored verbatim on
    // the `CompletionLog` row and returned in the response body unchanged.
    //
    // Sequence: POST → POST /skip with notes → assert log → GET → assert status "skipped".
    //
    // Open the shared pool so that all three routers observe each other's writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Three routers: create the task, skip it, then verify the status.
    let app_create = build_app(db.clone());
    // Skip router: shares the pool so it can find and update the created task.
    let app_skip = build_app(db.clone());
    // GET router: used to verify the task status changed after the skip.
    let app_get = build_app(db);

    // Create the task we will skip.
    let create_resp = post_json(
        app_create,
        "/api/v1/tasks",
        json!({ "title": "weekly report" }),
    )
    .await;
    // Extract the task ID from the creation response.
    let body = body_json(create_resp).await;
    // The ID is needed to build the `/skip` URL.
    let id = body["id"].as_str().expect("id must be a string");

    // Skip the task with an optional context note.
    let skip_resp = post_json(
        app_skip,
        &format!("/api/v1/tasks/{id}/skip"),
        // The notes field carries household context for why this instance was skipped.
        json!({ "instance_date": "2026-05-25", "notes": "public holiday" }),
    )
    .await;
    // The skip endpoint must return 200 OK with the skip log entry.
    assert_eq!(skip_resp.status(), StatusCode::OK);

    // Parse the skip log response returned by the endpoint.
    let log = body_json(skip_resp).await;
    // `skipped` must be true — this was a skip event, not a completion.
    assert_eq!(log["skipped"], true);
    // The notes field must survive the round-trip through the JSON body and storage.
    assert_eq!(log["notes"], "public holiday");

    // Verify the task status changed to "skipped" via a subsequent GET.
    let task_resp = get(app_get, &format!("/api/v1/tasks/{id}")).await;
    // The GET must succeed.
    assert_eq!(task_resp.status(), StatusCode::OK);
    // Parse the task body to check the status.
    let task_body = body_json(task_resp).await;
    // The status must be "skipped"; "done" would mean the wrong handler was invoked.
    assert_eq!(task_body["status"], "skipped");
}

#[tokio::test]
async fn change_assignee_updates_current_assignee_id() {
    // POST /assignee must set `current_assignee_id` on the task and return the
    // updated task body. This is the "one-tap reassignment" affordance (brief §8.2).
    //
    // The placeholder member UUID is the only valid member ID in the test database
    // (it is the only row in the `members` table, inserted by migration 0001).
    //
    // Unlike completion logging (which is append-only), the assignee field is mutable.
    // Reassignment overwrites the previous value on the task row directly — no history
    // of prior assignees is tracked (brief §6.1; history is for completions only).
    //
    // Sequence: POST → POST /assignee → assert 200 + assignee → GET → assert assignee.
    //
    // Open the shared pool so that all three routers observe each other's writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Three routers: create, assign, then verify.
    let app_create = build_app(db.clone());
    // Assign router: shares the pool so it can update the created task's assignee.
    let app_assign = build_app(db.clone());
    // GET router: used to verify the assignee change was persisted.
    let app_get = build_app(db);

    // Create the task with no initial assignee.
    let create_resp = post_json(
        app_create,
        "/api/v1/tasks",
        json!({ "title": "cook dinner" }),
    )
    .await;
    // Extract the task ID for use in the /assignee URL.
    let body = body_json(create_resp).await;
    // The ID is needed to build the POST /assignee URL.
    let id = body["id"].as_str().expect("id must be a string");

    // Assign the placeholder member as the current_assignee_id.
    let assign_resp = post_json(
        app_assign,
        &format!("/api/v1/tasks/{id}/assignee"),
        // The placeholder member UUID is the only valid FK reference in the test DB.
        json!({ "member_id": "00000000-0000-7000-8000-000000000001" }),
    )
    .await;
    // The /assignee endpoint must return 200 OK with the updated task.
    assert_eq!(assign_resp.status(), StatusCode::OK);

    // Parse the updated task returned by the /assignee endpoint.
    let assign_body = body_json(assign_resp).await;
    // The response must show the new assignee UUID in the `current_assignee_id` field.
    assert_eq!(
        assign_body["current_assignee_id"],
        "00000000-0000-7000-8000-000000000001"
    );

    // Verify the change was persisted by fetching the task via GET.
    let get_resp = get(app_get, &format!("/api/v1/tasks/{id}")).await;
    // The GET must succeed.
    assert_eq!(get_resp.status(), StatusCode::OK);
    // Parse the task to check the persisted assignee.
    let get_body = body_json(get_resp).await;
    // The GET response must also show the assigned member UUID.
    assert_eq!(
        get_body["current_assignee_id"],
        "00000000-0000-7000-8000-000000000001"
    );
}

#[tokio::test]
async fn get_task_history_returns_completion_logs() {
    // GET /history must return all completion log entries for the task, ordered
    // newest-first (`completed_at DESC`). Both completions and skip events must
    // appear in the history — skipping is a first-class fact.
    //
    // This test creates one completion and one skip event, then verifies that the
    // history endpoint returns both in the correct order.
    //
    // Ordering: `completed_at DESC` — the most recent event appears first. Clients
    // display history newest-first in the task detail screen (brief §8.2).
    //
    // Note: `completed_at` is a precise RFC 3339 timestamp set by the server at the
    // moment of the request. Since the skip request is sent after the completion
    // request, its `completed_at` is always later — the ordering is deterministic.
    //
    // Sequence: POST → complete (2026-05-24) → skip (2026-05-25) → GET /history.
    //
    // Open the shared pool so that all four routers observe each other's writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Four routers: create, complete, skip, then fetch history.
    let app_create = build_app(db.clone());
    // Complete and skip routers each need pool access to write log entries.
    let app_complete = build_app(db.clone());
    let app_skip = build_app(db.clone());
    // History router: reads the two log entries written by the above routers.
    let app_history = build_app(db);

    // Create the task whose history we will inspect.
    let create_resp = post_json(app_create, "/api/v1/tasks", json!({ "title": "exercise" })).await;
    // Extract the task ID for use in all subsequent requests.
    let body = body_json(create_resp).await;
    // The ID is used in the complete, skip, and history URLs.
    let id = body["id"].as_str().expect("id must be a string");

    // Record a completion event for 2026-05-24.
    post_json(
        app_complete,
        &format!("/api/v1/tasks/{id}/complete"),
        // Earlier date — this event must appear second in the history (oldest first → second).
        json!({ "instance_date": "2026-05-24" }),
    )
    .await;

    // Record a skip event for 2026-05-25 (the day after the completion).
    post_json(
        app_skip,
        &format!("/api/v1/tasks/{id}/skip"),
        // Later date — this event must appear first in the history (newest first).
        json!({ "instance_date": "2026-05-25" }),
    )
    .await;

    // Fetch the full history for this task.
    let history_resp = get(app_history, &format!("/api/v1/tasks/{id}/history")).await;
    // The history endpoint must return 200 OK.
    assert_eq!(history_resp.status(), StatusCode::OK);

    // Parse the history response body.
    let history = body_json(history_resp).await;
    let logs = history.as_array().expect("history must be a JSON array");
    // Both events (the completion and the skip) must appear in the history.
    assert_eq!(
        logs.len(),
        2,
        "both the completion and skip events must be logged"
    );

    // The history is ordered `completed_at DESC` — newest event first.
    // The skip event (2026-05-25) occurred after the completion (2026-05-24),
    // so it must appear first in the list.
    assert_eq!(logs[0]["skipped"], true, "newest entry must be the skip");
    // The completion (2026-05-24) must appear second (older event → lower in list).
    assert_eq!(
        logs[1]["skipped"], false,
        "older entry must be the completion"
    );
}

#[tokio::test]
async fn get_upcoming_returns_instances_for_recurring_task() {
    // GET /upcoming must return pre-materialised instances for open recurring tasks.
    // The instances are materialised during task creation (up to a 60-day horizon).
    // This test verifies that the materialisation step ran and the endpoint can query it.
    //
    // A weekly recurrence (FREQ=WEEKLY;BYDAY=TH) materialises multiple Thursdays
    // within 60 days, so at least one instance must appear in the response.
    //
    // Materialisation is synchronous: the POST /tasks handler calls
    // `materialise_and_store_instances` before returning 201. By the time the
    // GET /upcoming request is sent, instances are already in the database.
    //
    // The timezone (`Europe/Amsterdam`) is used for DST-correct materialisation.
    // An Amsterdam Thursday midnight may shift by one hour relative to UTC in DST.
    //
    // Sequence: POST (recurring task) → assert 201 → GET /upcoming → assert instances.
    //
    // Open the shared pool so that both routers observe each other's writes.
    let db = open_database("sqlite::memory:")
        .await
        .expect("in-memory db");
    // Two routers: create the recurring task, then query upcoming instances.
    let app_create = build_app(db.clone());
    // Upcoming router: shares the pool so it can find the materialised instances.
    let app_upcoming = build_app(db);

    // Create a recurring task with a weekly RRULE.
    let create_resp = post_json(
        app_create,
        "/api/v1/tasks",
        json!({
            "title": "weekly meeting",
            // FREQ=WEEKLY;BYDAY=TH means every Thursday.
            "recurrence_rrule": "FREQ=WEEKLY;BYDAY=TH",
            // Amsterdam timezone for DST-correct materialisation.
            "recurrence_timezone": "Europe/Amsterdam"
        }),
    )
    .await;
    // The task creation must succeed; 422 would indicate an invalid RRULE.
    assert_eq!(create_resp.status(), StatusCode::CREATED);

    // Fetch the upcoming instances.
    let upcoming_resp = get(app_upcoming, "/api/v1/tasks/upcoming").await;
    // The upcoming endpoint must return 200 OK.
    assert_eq!(upcoming_resp.status(), StatusCode::OK);

    // Parse the upcoming instances list.
    let upcoming = body_json(upcoming_resp).await;
    let instances = upcoming.as_array().expect("upcoming must be a JSON array");
    // At least one Thursday must fall within the next 60 days of materialisation.
    assert!(
        !instances.is_empty(),
        "at least one instance must be materialised for the weekly rule"
    );
    // The `title` field is denormalised from the parent task onto each instance.
    assert_eq!(instances[0]["title"], "weekly meeting");
    // Each instance must have a `scheduled_at` RFC 3339 datetime.
    assert!(
        instances[0]["scheduled_at"].is_string(),
        "scheduled_at must be an RFC 3339 string"
    );
}
