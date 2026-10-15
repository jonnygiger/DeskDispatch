## 2025-03-10 - [Concurrent schedule option queries with tokio::join!]
**Learning:** Schedule creation and edit form handlers were sequentially querying automations and worker group options, incurring 2 consecutive database round-trips before rendering or handling form errors.
**Action:** Use `tokio::join!` in a helper function (`fetch_schedule_options`) to run independent selection option queries concurrently against the PgPool, roughly halving query wait latency.

## 2025-03-10 - [Batching automation step subtype queries in worker API]
**Learning:** `fetch_full_automation_json` previously queried each step's subtype table (`step_mouse_clicks`, `step_key_presses`, `step_find_pixel_rgb`, `step_find_bitmap`, `step_branches`) inside a loop over `automation_steps`, resulting in an N+1 database query bottleneck whenever worker nodes polled `/api/v1/workers/next-assignment` or retrieved task run details.
**Action:** When serializing step hierarchy payloads, JOIN subtype detail tables with `automation_steps` on `automation_id` to bulk-fetch subtype rows into HashMaps in fixed O(1) bulk queries before building the JSON output.

## 2025-03-10 - [Concurrent worker management queries with tokio::join!]
**Learning:** `get_workers_handler` in `src/routes/workers.rs` sequentially executed 4 independent queries (`task_worker_pcs`, `worker_group_members` mappings, `worker_groups`, and member display names) across 4 database round-trips before building worker and group list items.
**Action:** Use `tokio::join!` to execute independent worker list and membership option queries concurrently across PgPool connections, reducing DB wait latency for worker management views by ~75%.

## 2025-03-10 - [Eliminating N+1 database queries on automation detail view]
**Learning:** `get_automation_detail_handler` in `src/routes/automations/detail.rs` previously ran `fetch_step_description` sequentially for every step in an automation, triggering per-step subtype and target label database queries and resulting in 4 + 3N sequential DB round trips per request.
**Action:** Create a bulk step view helper `fetch_automation_step_views` in `src/routes/automations/steps.rs` that bulk-fetches active step subtype details into HashMaps concurrently with `tokio::join!` and resolves branch step labels in memory. Execute step views, worker groups, and parameter options concurrently using `tokio::join!`, reducing database round trips to 3 fixed calls regardless of step count.

## 2025-03-10 - [Concurrent task run detail and recording session queries with tokio::join!]
**Learning:** `get_run_detail_handler` in `src/routes/runs.rs` and recording handlers in `src/routes/recordings.rs` sequentially executed independent database queries for the primary entity record, executed step/event rows, and variable values in 2 to 3 consecutive round-trips.
**Action:** Use `tokio::join!` to execute independent entity, step/event, and variable value queries concurrently across PgPool connections, reducing DB wait latency for task run detail views by ~66% and recording session views by ~50%.

## 2025-03-10 - [Concurrent automation sub-view queries with tokio::join!]
**Learning:** `get_automation_bitmaps_handler`, `get_automation_variables_handler`, and `get_automation_parameters_handler` sequentially executed independent queries for the target automation record and its associated sub-items (bitmaps, variables, or parameters), incurring 2 consecutive database round-trips per request.
**Action:** Use `tokio::join!` to execute the automation metadata query and associated items list query concurrently over the PgPool, halving database wait latency for automation sub-views (~50% reduction).

## 2025-03-10 - [Concurrent step edit detail and dropdown option queries with tokio::join!]
**Learning:** `get_edit_step_handler` in `src/routes/automations/steps.rs` sequentially queried step subtype details (`step_find_bitmap`, `step_branches`, `step_find_pixel_rgb`, `step_mouse_clicks`) and then fetched dropdown option lists (`fetch_available_bitmaps`, `fetch_automation_variables`, `fetch_automation_step_options`), taking 2 sequential DB round-trips per step edit form load.
**Action:** Use `tokio::join!` to execute step subtype detail queries concurrently with form dropdown option queries across PgPool connections, halving database wait latency (~50% reduction) on step editing UI endpoints.

## 2025-03-10 - [Concurrent manual run trigger queries with tokio::join!]
**Learning:** `post_run_now_automation_handler` in `src/routes/automations/detail.rs` sequentially executed 4 database queries (`SELECT status`, `validate_automation_for_activation`, `SELECT COUNT(*)`, and `SELECT automation_parameters`) before queueing a task run.
**Action:** Use `tokio::join!` to execute the automation status, step count, and parameter definition queries concurrently across PgPool connections, reducing pre-dispatch database wait latency on manual run triggers from 4 sequential round-trips down to 2.

## 2025-03-10 - [Zero-allocation SQLx deserialization for UserRole]
**Learning:** `get_users_handler` in `src/routes/users.rs` and `SessionUserRow` in `src/auth/user.rs` were querying role as `String` and calling `UserRole::from_str` per row, causing unnecessary heap allocations and string parsing overhead on user listing and authentication extractor paths.
**Action:** Derive `sqlx::Type` on `UserRole` and `sqlx::FromRow` on `UserListItem` / `SessionUserRow`, enabling direct `sqlx::query_as::<_, UserListItem>` deserialization from database `text` columns without intermediate `String` allocations.
