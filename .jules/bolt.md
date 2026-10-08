## 2025-03-10 - [Concurrent schedule option queries with tokio::join!]
**Learning:** Schedule creation and edit form handlers were sequentially querying automations and worker group options, incurring 2 consecutive database round-trips before rendering or handling form errors.
**Action:** Use `tokio::join!` in a helper function (`fetch_schedule_options`) to run independent selection option queries concurrently against the PgPool, roughly halving query wait latency.

## 2025-03-10 - [Batching automation step subtype queries in worker API]
**Learning:** `fetch_full_automation_json` previously queried each step's subtype table (`step_mouse_clicks`, `step_key_presses`, `step_find_pixel_rgb`, `step_find_bitmap`, `step_branches`) inside a loop over `automation_steps`, resulting in an N+1 database query bottleneck whenever worker nodes polled `/api/v1/workers/next-assignment` or retrieved task run details.
**Action:** When serializing step hierarchy payloads, JOIN subtype detail tables with `automation_steps` on `automation_id` to bulk-fetch subtype rows into HashMaps in fixed O(1) bulk queries before building the JSON output.

## 2025-03-10 - [Concurrent worker management queries with tokio::join!]
**Learning:** `get_workers_handler` in `src/routes/workers.rs` sequentially executed 4 independent queries (`task_worker_pcs`, `worker_group_members` mappings, `worker_groups`, and member display names) across 4 database round-trips before building worker and group list items.
**Action:** Use `tokio::join!` to execute independent worker list and membership option queries concurrently across PgPool connections, reducing DB wait latency for worker management views by ~75%.
