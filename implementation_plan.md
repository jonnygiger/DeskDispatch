## Phase 1: Project Scaffolding & Infrastructure

**1.** Initialize the Cargo binary crate to serve as the foundation for the Rust server.
**2.** Add `axum` and `tokio` (full features) to the project dependencies for async web routing.
**3.** Add `sqlx` to dependencies with `postgres`, `runtime-tokio-rustls`, and `macros` features enabled.
**4.** Add template and data handling libraries: `askama`, `serde`, and `serde_json`.
**5.** Add utility and security crates: `argon2`, `uuid`, `dotenvy`, and `croner`.
**6.** Add observability and object storage crates: `tracing`, `tracing-subscriber`, and `aws-sdk-s3`.
**7.** Create a `docker-compose.yml` file defining a `postgres:16` service for local development.
**8.** Add a `minio/minio` service to `docker-compose.yml` configured with a one-shot bucket-creation script.
**9.** Create a `config.rs` module to parse environment variables for the database URL, S3 credentials, session keys, and bind address.
**10.** Install and wire up `sqlx-cli`, creating the `migrations/` directory and an initial empty migration.
**11.** Initialize `tracing_subscriber` to output pretty logs in development and JSON in production.
**12.** Implement the initial `main.rs` to build the `axum` Router and instantiate a `PgPoolOptions` connection pool.
**13.** Implement a `GET /healthz` endpoint that returns 200 OK only after successfully completing a live database round-trip.
**14.** Draft a `README.md` documenting the Docker, SQLx, and Cargo commands required to spin up the environment.

## Phase 2: Database Schema & Migrations

**15.** Create the SQL migration defining the `users` and `sessions` tables.
**16.** Create the SQL migration defining the `audit_log` table.
**17.** Create the SQL migration defining the core `automations` table.
**18.** Create the SQL migration defining the `automation_variables` and `automation_parameters` tables.
**19.** Create the SQL migration defining the `bitmaps` reference table.
**20.** Create the SQL migration defining the base `automation_steps` table.
**21.** Create the SQL migration defining the `step_mouse_clicks` and `step_key_presses` detail tables.
**22.** Create the SQL migration defining the `step_find_pixel_rgb` and `step_find_bitmap` detail tables.
**23.** Create the SQL migration defining the `step_branches` and `step_screenshots` detail tables.
**24.** Create the SQL migration defining the `task_worker_pcs`, `worker_groups`, and `worker_group_members` tables.
**25.** Create the SQL migration defining the `schedules` table.
**26.** Create the SQL migration defining the `task_runs`, `task_run_steps`, and `task_run_variable_values` tables.
**27.** Create the SQL migration defining the `recording_sessions` and `recording_events` tables.
**28.** Add CHECK constraints to the schema to enforce discriminated-union patterns on step branches.
**29.** Add CHECK constraints enforcing literal-vs-variable mutually exclusive pairs across all step detail tables.
**30.** Create database indexes targeting `idx_steps_automation_position`, `idx_schedules_due`, and `idx_task_runs_queued`.
**31.** Create a `seed` binary script to insert the initial admin user using credentials passed via environment variables.
**32.** Write a compile-time-checked `sqlx::query!` smoke test to validate that SQL macros resolve correctly against the live schema.

## Phase 3: Authentication, Security & RBAC

**33.** Implement the `POST /login` endpoint utilizing `argon2id` for password verification.
**34.** Implement session generation, database persistence, and HttpOnly `SameSite=Lax` cookie delivery.
**35.** Implement the `POST /logout` endpoint to destroy active sessions.
**36.** Create an `AuthUser` extractor to load session data from cookies and redirect unauthorized requests to `/login`.
**37.** Implement a Role-Based Access Control (RBAC) middleware layer enforcing viewer, editor, and admin permissions.
**38.** Implement server-side generation of per-session CSRF tokens.
**39.** Create an Askama template macro to automatically inject CSRF tokens into HTML forms.
**40.** Implement a `tower` layer to validate CSRF tokens on all incoming POST requests.
**41.** Implement login rate-limiting logic grouping attempts by client IP and username.
**42.** Implement `GET /account/password` and `POST /account/password` handlers for self-service credential updates.

## Phase 4: Web Application Shell

**43.** Create the base Askama `layout.html` template defining the structural shell.
**44.** Implement the top navigation bar with role-conditional rendering logic.
**45.** Implement custom 404 and 500 error pages extending the base layout.
**46.** Configure a static-asset route using `rust-embed` to serve hand-written, zero-JS CSS.
**47.** Build the base `GET /` Dashboard route.
**48.** Implement the four high-level summary statistics tiles on the Dashboard.
**49.** Implement the Recent Runs dashboard table to query and display data from the underlying tables.
**50.** Implement the Recent Workers dashboard table to query and display data from the underlying tables.

## Phase 5: Automations CRUD & Ordering Logic

**51.** Implement the `GET /automations` handler and view to list records with a status filter.
**52.** Implement the creation form and POST handler for new automations.
**53.** Implement handlers for renaming automations and updating their operational status.
**54.** Implement the automation delete/archive handler including user confirmation flows.
**55.** Build the automation editor UI shell that renders associated steps ordered by their `position` integer.
**56.** Implement the simple `key_press` step end-to-end to establish the form payload pipeline.
**57.** Implement the sparse-`position` calculation algorithm to handle midpoint insertions and end-of-list appends.
**58.** Implement `move-up` and `move-down` positional mutation handlers for reordering steps.
**59.** Implement step deletion logic and sparse position compaction for when positional gaps become too narrow.

## Phase 6: Object Storage & Zoom Component

**60.** Initialize the S3/MinIO client wrapper for generating presigned GET URLs.
**61.** Implement presigned PUT URL generation for worker node file uploads.
**62.** Implement presigned POST policy generation for browser-direct file uploads.
**63.** Build the reusable Askama partial defining the three-panel image magnifier component.
**64.** Implement server-computed CSS background positioning and sizing math for the `shot--normal` and `shot--zoom400` panels.
**65.** Apply `image-rendering: pixelated` styling and grid overlays for the `shot--grid` panel.
**66.** Implement dynamic coordinate math to correctly position the CSS-triangle marker across all three scales.
**67.** Build `GET /media/screenshots/{id}` and `GET /media/bitmaps/{id}` routes that issue 302 redirects to presigned S3 URLs.
**68.** Create a throwaway internal route to manually verify the CSS zoom and pan mathematics of the magnifier component.

## Phase 7: Bitmap Library

**69.** Implement the `GET /bitmaps` list page, displaying image thumbnails using the magnifier component.
**70.** Implement the bitmap upload form, pushing files directly to object storage via presigned POST policies.
**71.** Implement the handler to delete bitmaps from both the database and object storage.
**72.** Build the zero-JS two-click region picker logic using `<input type="image">` to capture the coarse top-left corner.
**73.** Extend the region picker logic to capture the precise bottom-right corner using the grid panel view.
**74.** Implement a confirm/re-pick crop preview step prior to committing the region as a saved bitmap.

## Phase 8: Point-Based Step Types & Pickers

**75.** Implement server-side mapping logic to translate coarse clicks on a fit-to-width screenshot into native pixel approximations.
**76.** Implement coordinate refinement logic mapping fine clicks on the grid panel to exact native coordinates.
**77.** Implement the `mouse_click` step creation and edit forms, binding the X/Y fields and click-type selectors.
**78.** Implement the `find_pixel_rgb` step creation and edit forms.
**79.** Build the `+ New Step` selection interface offering the available step primitives.
**80.** Implement plain-language string generators summarizing `mouse_click` and `find_pixel_rgb` instructions for the UI.

## Phase 9: Complex Step Types

**81.** Implement the `find_bitmap` step form allowing users to select an image from the established bitmap library.
**82.** Add functionality to the `find_bitmap` form allowing ad-hoc regional selection and search-region bounding.
**83.** Build the `branch` step form shell featuring a condition-type toggle between pixel and bitmap evaluations.
**84.** Implement branching target selectors populated dynamically with foreign keys pointing to existing steps in the current automation.
**85.** Implement plain-language string generation outlining "if X → Step A, else → Step B" routing logic for branch steps.

## Phase 10: Variables & Dynamic Binding

**86.** Implement CRUD UI and handlers for `automation_variables`.
**87.** Implement CRUD UI and handlers for `automation_parameters`, including default value constraints.
**88.** Implement literal-vs-reference UI toggles on input fields across the step authoring forms.
**89.** Build form-level validation logic to strictly enforce the "exactly one of literal or reference" rule.
**90.** Wire the output of `find_pixel_rgb` steps to populate selected `automation_variables`.
**91.** Wire the output of `find_bitmap` steps to populate selected `automation_variables`.
**92.** Update the step card renderer to display referenced variable names in place of hardcoded coordinates.

## Phase 11: Worker API Foundations

**93.** Implement administrative CRUD interfaces for managing `task_worker_pcs` and `worker_groups`.
**94.** Implement UI to generate one-time registration tokens for new worker nodes.
**95.** Implement administrative handlers to deactivate workers and rotate API keys.
**96.** Build the `POST /api/v1/workers/register` endpoint to exchange registration tokens for persistent, hashed API keys.
**97.** Create a worker-auth extractor to authorize incoming API requests via SHA-256 hash lookups.
**98.** Implement the `POST /api/v1/workers/heartbeat` endpoint for online status tracking.
**99.** Implement the foundational `GET /api/v1/workers/next-assignment` long-polling endpoint configured to return a `none` payload.

## Phase 12: Execution Engine & Dispatch

**100.** Implement the `POST /automations/{id}/run-now` handler to inject manual task runs into the queue.
**101.** Implement a `SELECT … FOR UPDATE SKIP LOCKED` query within the `next-assignment` endpoint to claim queued runs concurrently.
**102.** Format the dispatch response to deliver the complete automation JSON payload to the worker.
**103.** Implement the `GET /task-runs/{id}` API endpoint allowing workers to resume execution.
**104.** Implement the `POST /step-result` and `POST /complete` API endpoints for workers to report progress.
**105.** Integrate screenshot upload URL generation and the `POST /screenshots/{key}/commit` endpoint into the worker lifecycle.
**106.** Implement a scheduled sweeper task to transition stalled task runs with silent heartbeats to a `lost` status.
**107.** Build the filterable `GET /runs` list view UI.
**108.** Build the `GET /runs/{id}` step-by-step detail view, utilizing the magnifier component to display captured runtime values.
**109.** Implement the `POST /runs/{id}/cancel` handler to abort running executions.

## Phase 13: Recording Capture & Conversion

**110.** Extend the `next-assignment` endpoint to dispatch `start_recording` instructions to targeted workers.
**111.** Implement the `POST /recordings/{id}/events` endpoint to ingest batched desktop actions from the worker.
**112.** Implement the `POST /recordings/{id}/stop` endpoint to finalize a recording session.
**113.** Build a live-status web UI utilizing a meta refresh tag and a manual Stop button.
**114.** Build a recording review UI displaying event cards with toggleable Exclusion checkboxes.
**115.** Implement the conversion engine to bulk-generate draft automations, steps, and screenshots from recorded event sequences.

## Phase 14: Scheduling Engine

**116.** Implement the `schedules` list view UI.
**117.** Implement the schedule creation and edit forms featuring cron-expression inputs and format hints.
**118.** Integrate `croner` to provide server-side validation and inline error rendering for malformed cron inputs.
**119.** Implement the core scheduling tick interval using `tokio::time::interval`.
**120.** Implement the scheduled database sweep to queue tasks for due schedules and recalculate their next run times.
**121.** Update the dispatch claim logic to correctly evaluate `worker_group_id` targeting for scheduled executions.

## Phase 15: Hardening, Observability & Deployment

**122.** Extend the `audit_log` middleware to capture all mutation handlers globally across the application.
**123.** Conduct an `EXPLAIN` query pass to verify that dispatch, scheduling, and positional queries correctly hit their intended indexes.
**124.** Wrap all primary request handlers, database calls, and S3 interactions in structured `tracing` spans.
**125.** Create a multi-stage `Dockerfile` optimizing the final binary size and layer caching for Rust.
**126.** Create a production-ready `docker-compose.yml` integrating the application, Postgres, MinIO, and a migration startup container.
**127.** Implement `sqlx::test`-backed integration tests validating step positioning and compaction math.
**128.** Implement `sqlx::test`-backed integration tests verifying concurrent dispatch claiming and stale-run eviction.
**129.** Execute a final manual QA pass validating every UI route, form submission, and execution flow end-to-end.
