# DeskDispatch — GUI Automation Task Server

DeskDispatch is a high-performance, zero-JavaScript task server for managing and orchestrating GUI automations across headless task-worker PCs. Built with Rust, Axum, SQLx, PostgreSQL, Askama, and S3-compatible object storage (RustFS), DeskDispatch provides an operator web interface and a lightweight HTTP/JSON polling API for worker machines.

---

## Table of Contents

- [Overview & Architecture](#overview--architecture)
- [Technology Stack](#technology-stack)
- [Zero-JavaScript Interface Design](#zero-javascript-interface-design)
- [Prerequisites & Environment Configuration](#prerequisites--environment-configuration)
- [Getting Started & Local Development](#getting-started--local-development)
  - [1. Infrastructure Services](#1-infrastructure-services)
  - [2. Running Database Migrations](#2-running-database-migrations)
  - [3. Seeding the Initial Admin User](#3-seeding-the-initial-admin-user)
  - [4. Running the Task Server](#4-running-the-task-server)
  - [5. Running Tests](#5-running-tests)
- [Web Operator Interface — Route Map](#web-operator-interface--route-map)
- [Worker-Facing API Reference](#worker-facing-api-reference)
- [Database Schema & Data Model](#database-schema--data-model)
- [Features & System Capabilities](#features--system-capabilities)
  - [Automations & Step Primitives](#automations--step-primitives)
  - [Sparse Floating-Point Step Ordering](#sparse-floating-point-step-ordering)
  - [Dynamic Variable & Parameter Binding](#dynamic-variable--parameter-binding)
  - [Reference Bitmaps & Zero-JS Region Picker](#reference-bitmaps--zero-js-region-picker)
  - [Three-Panel CSS Screenshot Magnifier](#three-panel-css-screenshot-magnifier)
  - [Worker PC & Group Management](#worker-pc--group-management)
  - [Minimum Worker Agent Version Enforcement](#minimum-worker-agent-version-enforcement)
  - [Execution & Concurrent Dispatch Engine](#execution--concurrent-dispatch-engine)
  - [Stalled Execution Sweeper & Timeout Recovery](#stalled-execution-sweeper--timeout-recovery)
  - [Automated Data Retention & Storage Maintenance](#automated-data-retention--storage-maintenance)
  - [Runs & Execution History UI](#runs--execution-history-ui)
  - [Worker Screenshots & Execution Capture](#worker-screenshots--execution-capture)
  - [Recording Session Capture & Conversion Engine](#recording-session-capture--conversion-engine)
  - [Scheduling & Cron Engine](#scheduling--cron-engine)
  - [User Management & Access Control](#user-management--access-control)
  - [Binary CLI Subcommands](#binary-cli-subcommands)
- [Security & Access Control](#security--access-control)
- [Implementation Roadmap & Plan Status](#implementation-roadmap--plan-status)

---

## Overview & Architecture

DeskDispatch serves as the central control plane for GUI automation workflows. Operator users define automations containing sequences of mouse clicks, keystrokes, pixel color checks, reference bitmap searches, and conditional branches. Headless worker PCs poll the task server over HTTPS/JSON to receive assignments, execute steps locally, and report execution results.

```
┌─────────────────────────────────────────┐
│     Operator Browser (HTML/CSS only)    │
└────────────────────┬────────────────────┘
                     │ HTTPS / Form POSTs
                     ▼
┌─────────────────────────────────────────┐
│      DeskDispatch Task Server           │
│         (Rust / Axum / Askama)          │
└──────────┬──────────────────┬───────────┘
           │                  │
    PostgreSQL              RustFS / S3
(Metadata, Steps,         (Screenshots &
 Runs, Workers)            Ref Bitmaps)
           ▲                  ▲
           │                  │
           └─────────┬────────┘
                     │ HTTPS / JSON Poll
┌────────────────────┴────────────────────┐
│         Task Worker PCs (Fleet)         │
└─────────────────────────────────────────┘
```

**Key Architectural Guarantees:**
- **Worker-Initiated Pull Model:** Workers poll the server via long-polling (`GET /api/v1/workers/next-assignment`). The server never opens inbound connections to worker PCs, enabling deployment across NATs and firewalls.
- **Zero-JavaScript Web Interface:** All web views are rendered on the server using Askama templates. Form submissions use standard HTTP POST with 303 Redirects. Interaction mechanisms like coordinate pickers leverage native HTML `<input type="image">`.
- **Atomic Dispatch:** Task run claiming utilizes PostgreSQL `SELECT ... FOR UPDATE SKIP LOCKED` queries to guarantee thread-safe, concurrent task distribution across multiple worker nodes without external locking services.

---

## Technology Stack

| Component | Technology | Rationale |
| --- | --- | --- |
| **Language** | Rust (2024 edition) | Strong type safety, zero-cost abstractions, memory safety, and high performance |
| **Web Server** | Axum & Tokio | Asynchronous HTTP routing, Tower middleware ecosystem, and non-blocking I/O |
| **Database & ORM** | PostgreSQL & SQLx | Async connection pooling, compile-time SQL query validation macro support |
| **Templating Engine** | Askama | Build-time compiled HTML templates with zero runtime parsing overhead |
| **Object Storage** | RustFS / S3 (aws-sdk-s3) | S3-compatible storage for reference bitmaps and execution screenshots |
| **Password Hashing** | Argon2id | Memory-hard key derivation function for operator passwords |
| **API Authentication** | SHA-256 Hashes | Fast, high-entropy hashing for machine worker bearer tokens |
| **Cron Math** | Croner | Standard 5-field cron parsing and next-run evaluation |
| **Asset Embedding** | rust-embed | Self-contained binary distribution including zero-JS CSS stylesheets |

---

## Zero-JavaScript Interface Design

DeskDispatch accomplishes rich interactive functionality entirely without client-side JavaScript:

1. **Three-Panel Screenshot Magnifier:**
   A single captured screenshot is rendered across three viewport panels (Normal Fit, 400% Zoom, 20x Pixel Grid) using CSS `background-image`, `background-size`, `background-position`, and `image-rendering: pixelated`. Marker triangles are calculated server-side and positioned via inline CSS styles.
2. **Native Coordinate Selection:**
   Two-pass coordinate picking relies on `<input type="image">`. A coarse click on a scaled screenshot posts the relative coordinates (`x` and `y`) to the server, which converts them to native screen space and renders a high-zoom pixel grid for fine adjustment.
3. **Region Crop Selection:**
   Two-step sequential clicking ("click top-left corner", "click bottom-right corner") creates cropped search region boundaries and reference bitmaps without requiring canvas or DOM manipulation.
4. **Form Mechanics & Navigation:**
   All mutations execute via `<form method="POST">` with hidden CSRF tokens, followed by a `303 See Other` redirect to maintain clean browser history and prevent duplicate form submission on refresh.

---

## Prerequisites & Environment Configuration

### Prerequisites
- **Rust Toolchain:** 1.85+ (2024 edition support)
- **Docker & Docker Compose:** For running PostgreSQL and RustFS object storage locally

### Environment Variables (`.env`)

```env
# Database
DATABASE_URL=postgres://postgres:postgrespassword@localhost:5432/deskdispatch

# Object Storage (S3 / RustFS)
S3_ENDPOINT=http://localhost:9000
S3_BUCKET=deskdispatch-bucket
S3_ACCESS_KEY=rustfsadmin
S3_SECRET_KEY=rustfsadminpassword
S3_REGION=us-east-1

# Application Security & Server Config
SESSION_SECRET=super-secret-key-change-me
BIND_ADDRESS=0.0.0.0:3000
APP_ENV=development
MIN_AGENT_VERSION=1.0.0
```

---

## Getting Started & Local Development

### 1. Infrastructure Services

Start the local PostgreSQL database and RustFS object storage containers:

```bash
docker compose up -d
```

This provisions:
- **PostgreSQL:** `localhost:5432` (db: `deskdispatch`, user: `postgres`, pass: `postgrespassword`)
- **RustFS S3 Service:** `http://localhost:9000` (Console on `http://localhost:9001`)
- **Bucket Creation:** Initializes `deskdispatch-bucket` automatically on startup.

### 2. Running Database Migrations

Migrations run automatically on application boot. You can also run migrations manually or inspect schema files in `migrations/`:

```bash
# Migrations are embedded and executed automatically via main.rs
```

### 3. Seeding the Initial Admin User

Run the included seed binary to create the default administrative account:

```bash
ADMIN_USERNAME=admin ADMIN_PASSWORD=adminpassword ADMIN_DISPLAY_NAME="Administrator" cargo run --bin seed
```

### 4. Running the Task Server

Start the Axum web server:

```bash
cargo run
```

Access the health check endpoint to confirm operational status:

```bash
curl http://localhost:3000/healthz
# Returns 200 OK: "OK"
```

Open `http://localhost:3000` in your browser to log in.

### 5. Running Tests

Execute internal library unit tests:

```bash
cargo test --lib
```

---

## Web Operator Interface — Route Map

All GET routes render Askama HTML templates; all POST routes mutate state and issue HTTP 303 redirects.

| Area | Route Path | Method | Minimum Role | Status | Description |
| --- | --- | --- | --- | --- | --- |
| **System** | `/healthz` | GET | Public | Implemented | Health check endpoint |
| | `/static/{*path}` | GET | Public | Implemented | Embedded static CSS assets |
| | `/dev/magnifier-verify` | GET | Public | Implemented | Magnifier CSS verification tool |
| **Auth** | `/login` | GET, POST | Public | Implemented | Session login with rate limiting |
| | `/logout` | POST | Authenticated | Implemented | Destroy active session |
| | `/account/password` | GET, POST | Authenticated | Implemented | Self-service password change |
| **Dashboard** | `/` | GET | Authenticated | Implemented | Overview stats & recent runs/workers |
| **Automations**| `/automations` | GET, POST | Authenticated (POST: Editor) | Implemented | List & create automations |
| | `/automations/new` | GET | Editor | Implemented | Automation creation form |
| | `/automations/{id}` | GET, POST | Authenticated (POST: Editor) | Implemented | Step editor & metadata update |
| | `/automations/{id}/run-now` | POST | Editor | Implemented | Trigger immediate manual run |
| | `/automations/{id}/delete` | GET, POST | Editor | Implemented | Delete automation confirmation |
| **Steps** | `/automations/{id}/steps/new` | GET | Editor | Implemented | Step type selection picker |
| | `/automations/{id}/steps/new/{type}` | GET | Editor | Implemented | Step creation form (`mouse_click`, `key_press`, `find_pixel_rgb`, `find_bitmap`, `branch`) |
| | `/automations/{id}/steps` | POST | Editor | Implemented | Submit new step |
| | `/automations/{id}/steps/{sid}/edit` | GET | Editor | Implemented | Step edit form |
| | `/automations/{id}/steps/{sid}` | POST | Editor | Implemented | Update existing step |
| | `/automations/{id}/steps/{sid}/move-up` | POST | Editor | Implemented | Swap step position upward |
| | `/automations/{id}/steps/{sid}/move-down`| POST | Editor | Implemented | Swap step position downward |
| | `/automations/{id}/steps/{sid}/delete` | POST | Editor | Implemented | Delete step |
| **Variables & Params** | `/automations/{id}/variables` | GET, POST | Authenticated (POST: Editor) | Implemented | Automation variables CRUD |
| | `/automations/{id}/variables/{vid}` | POST | Editor | Implemented | Update variable |
| | `/automations/{id}/variables/{vid}/delete` | POST | Editor | Implemented | Delete variable |
| | `/automations/{id}/parameters` | GET, POST | Authenticated (POST: Editor) | Implemented | Automation parameters CRUD |
| | `/automations/{id}/parameters/{pid}` | POST | Editor | Implemented | Update parameter |
| | `/automations/{id}/parameters/{pid}/delete` | POST | Editor | Implemented | Delete parameter |
| **Bitmaps** | `/bitmaps` | GET, POST | Authenticated (POST: Editor) | Implemented | Shared reference bitmap library |
| | `/bitmaps/{id}/delete` | POST | Editor | Implemented | Delete bitmap from DB & S3 |
| | `/automations/{id}/bitmaps` | GET, POST | Authenticated (POST: Editor) | Implemented | Scoped automation bitmap library |
| | `/automations/{id}/bitmaps/{bid}/delete` | POST | Editor | Implemented | Delete scoped bitmap |
| | `/bitmaps/commit` | GET | Editor | Implemented | Probe image dimensions & finalize bitmap |
| | `/bitmaps/pick-region` | GET, POST | Editor | Implemented | Start region picker (top-left) |
| | `/bitmaps/pick-region/bottom-right` | POST | Editor | Implemented | Pick region bottom-right corner |
| | `/bitmaps/pick-region/confirm` | POST | Editor | Implemented | Confirm region crop |
| **Workers** | `/workers` | GET, POST | Admin | Implemented | List workers & create worker PC |
| | `/workers/new` | GET | Admin | Implemented | New worker creation form |
| | `/workers/{id}` | GET | Admin | Implemented | Worker PC detail view |
| | `/workers/{id}/edit` | GET, POST | Admin | Implemented | Edit worker details |
| | `/workers/{id}/delete` | POST | Admin | Implemented | Delete worker PC |
| | `/workers/{id}/deactivate` | POST | Admin | Implemented | Deactivate worker PC |
| | `/workers/{id}/rotate-key` | POST | Admin | Implemented | Issue new registration token |
| | `/worker-groups` | POST | Admin | Implemented | Create worker group |
| | `/worker-groups/new` | GET | Admin | Implemented | New worker group form |
| | `/worker-groups/{id}/edit` | GET, POST | Admin | Implemented | Edit worker group |
| | `/worker-groups/{id}/delete` | POST | Admin | Implemented | Delete worker group |
| **Schedules** | `/schedules` | GET, POST | Authenticated (POST: Editor) | Implemented | List & create schedules |
| | `/schedules/new` | GET | Editor | Implemented | Schedule creation form |
| | `/schedules/{id}/edit` | GET, POST | Editor | Implemented | Edit schedule & cron syntax |
| | `/schedules/{id}/delete` | POST | Editor | Implemented | Delete schedule |
| | `/schedules/{id}/toggle` | POST | Editor | Implemented | Enable/disable schedule |
| **Runs** | `/runs` | GET | Authenticated | Implemented | Filterable task run history |
| | `/runs/{id}` | GET | Authenticated | Implemented | Execution log & step screenshot detail |
| | `/runs/{id}/cancel` | POST | Editor | Implemented | Request cancellation of active run |
| **Recording** | `/workers/{id}/record` | GET | Editor | Implemented | Recording session worker selection |
| | `/workers/{id}/record/start` | POST | Editor | Implemented | Dispatch start recording instruction |
| | `/recordings/{id}` | GET | Authenticated | Implemented | Live status view (`meta refresh`) |
| | `/recordings/{id}/review` | GET | Editor | Implemented | Step-by-step event review UI |
| | `/recordings/{id}/convert` | POST | Editor | Implemented | Bulk-convert events into automation |
| | `/recordings/{id}/stop` | POST | Editor | Implemented | Request stop recording |
| **Media** | `/media/screenshots/{id}` | GET | Authenticated | Implemented | Redirect to presigned screenshot S3 URL |
| | `/media/bitmaps/{id}` | GET | Authenticated | Implemented | Redirect to presigned bitmap S3 URL |

---

## Worker-Facing API Reference

All endpoints except `/register` require HTTP Header `Authorization: Bearer <api_key>`. Machine API keys are authenticated via SHA-256 hash lookup against `task_worker_pcs.api_key_hash`.

| HTTP Endpoint | Status | Description |
| --- | --- | --- |
| `POST /api/v1/workers/register` | Implemented | Exchange single-use token (`registration_token` or `token`) for a permanent API key. |
| `POST /api/v1/workers/heartbeat` | Implemented | Worker heartbeat submitting system status, resolution, OS info, and current run ID. Returns `cancel_requested`. |
| `GET /api/v1/workers/next-assignment` | Implemented | Long-poll task dispatch endpoint. Claims queued runs matching worker groups via `FOR UPDATE SKIP LOCKED`. |
| `GET /api/v1/workers/task-runs/{id}` | Implemented | Fetch complete automation JSON payload to resume execution after worker restarts. |
| `POST /api/v1/workers/task-runs/{id}/step-result` | Implemented | Submit execution result for a single step, updating step results and dynamic variable values. |
| `POST /api/v1/workers/task-runs/{id}/complete` | Implemented | Finalize task run with status `succeeded` or `failed` and optional error message. |
| `GET /api/v1/workers/task-runs/{id}/screenshot-upload-url` | Implemented | Obtain short-lived presigned PUT URL for step execution screenshot. |
| `POST /api/v1/workers/task-runs/{id}/screenshots/{object_key}/commit` | Implemented | Commit uploaded screenshot dimensions and link to `task_run_steps`. |
| `POST /api/v1/workers/recordings/{session_id}/events` | Implemented | Push batched desktop events during active recording session. |
| `GET /api/v1/workers/recordings/{session_id}/screenshot-upload-url` | Implemented | Presigned S3 PUT URL for recording-time screenshot. |
| `POST /api/v1/workers/recordings/{session_id}/stop` | Implemented | Worker-initiated notification that recording stopped locally. |

---

## Database Schema & Data Model

DeskDispatch uses PostgreSQL with discriminated detail tables for automation step primitives:

```
                  ┌─────────────────┐
                  │   automations   │
                  └────────┬────────┘
                           │ 1:N
                           ▼
                  ┌─────────────────┐
                  │ automation_steps│ (position, step_type, post_delay_ms)
                  └────────┬────────┘
                           │ 1:1 Discriminated Foreign Key
    ┌──────────────────────┼──────────────────────┬──────────────────────┐
    ▼                      ▼                      ▼                      ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│step_mouse_clicks │ │step_key_presses  │ │step_find_pixel_rgb│ │ step_find_bitmap │
└──────────────────┘ └──────────────────┘ └──────────────────┘ └──────────────────┘
                           │
                           ▼
                  ┌──────────────────┐
                  │  step_branches   │ (condition_type, on_match_step_id, on_no_match_step_id)
                  └──────────────────┘
```

**Key Database Tables:**
- `users`: User identity, Argon2id password hash, role (`admin`, `editor`, `viewer`).
- `sessions`: Session UUIDs tied to users with HTTP-only session cookies.
- `audit_log`: Record of administrative and mutation actions.
- `automations`: Core automation metadata (name, description, status: `draft`/`active`/`archived`).
- `automation_steps`: Base table holding sparse float `position`, `step_type`, and `post_delay_ms`.
- `step_mouse_clicks`, `step_key_presses`, `step_find_pixel_rgb`, `step_find_bitmap`, `step_branches`: Detail tables enforcing literal-vs-reference variable pairs via `CHECK` constraints.
- `automation_variables` & `automation_parameters`: Dynamic runtime state bindings and default parameter values.
- `bitmaps` & `step_screenshots`: Reference images and editor screenshots stored in S3/RustFS.
- `task_worker_pcs`, `worker_groups`, `worker_group_members`: Worker registration, group assignments, SHA-256 API key hashes, and heartbeat status tracking.
- `task_runs`, `task_run_steps`, `task_run_variable_values`: Task execution queue, per-step execution logs, and captured runtime variable values.
- `schedules`: Cron schedule definitions and worker group targeting.
- `recording_sessions` & `recording_events`: Event streams captured from worker PCs for draft conversion.

---

## Features & System Capabilities

### Automations & Step Primitives
Automations consist of an ordered list of step primitives:
1. `mouse_click`: Click at specific X/Y coordinates or variable-bound coordinates (`left`, `right`, `middle`; `single`, `double`).
2. `key_press`: Execute single keys or chord combinations (e.g. `ctrl+alt+delete`, `F5`).
3. `find_pixel_rgb`: Sample pixel color at X/Y coordinates and optionally store captured color into an output variable.
4. `find_bitmap`: Search the screen for a reference bitmap within an optional bounding region and write found status and match coordinates to variables.
5. `branch`: Evaluate a pixel RGB condition or reference bitmap condition, branching to `on_match_step_id` or `on_no_match_step_id`.

Every step primitive includes an optional post-execution delay (`post_delay_ms`). Plain-language summary generators dynamically output human-readable instruction strings in the UI (e.g., *“Step 3 — Click «point_var» [left, single] · then wait 0.5s”*).

### Sparse Floating-Point Step Ordering
Step order is maintained via a `DOUBLE PRECISION` sparse `position` column. Inserting a step between position 10.0 and 20.0 assigns position 15.0 without requiring renumbering. If floating-point gaps become too tight after repeated midpoint insertions, the server automatically executes step position compaction to reset step gaps to clean increments.

To prevent floating-point precision exhaustion, the task server evaluates step gaps after step additions or movements (`check_and_compact_positions`). If the difference between any two consecutive step positions falls below `0.0001`, the server automatically executes step compaction (`compact_positions`). Compaction resets all step positions within the automation back to clean increments of `10.0` (10.0, 20.0, 30.0, etc.) in a single database transaction, preserving the exact original execution order while restoring wide position gaps for future step insertions.

### Dynamic Variable & Parameter Binding
Step parameters (coordinates, colors, thresholds) support either literal values or variable references (`automation_variables` / `automation_parameters`). Strict form-level and database `CHECK` constraints enforce that inputs contain *exactly one* literal value or variable binding. Output values from search steps (`find_pixel_rgb`, `find_bitmap`) populate variables for subsequent steps.

### Reference Bitmaps & Zero-JS Region Picker
Users can upload reference bitmaps directly to S3 via presigned POST policies or capture them directly from screenshots using the zero-JS region picker:
- First click selects the coarse top-left corner.
- Second click selects the bottom-right corner using a 20x zoomed grid panel view.
- Final preview step displays the cropped region for confirmation before persisting to the database and object storage.

### Three-Panel CSS Screenshot Magnifier
Step editor views and media previews feature a zero-JS three-panel CSS image magnifier component:
- **Normal Panel:** Scaled full-frame screenshot.
- **400% Zoom Panel:** 4x magnification focused around the target point.
- **20x Pixel Grid Panel:** High-magnification grid showing individual pixels separated by subtle 1px border lines.

All scaling and panning are driven by server-calculated inline CSS styles (`background-position`, `background-size`, `image-rendering: pixelated`).

### Worker PC & Group Management
Administrators manage worker fleets via the web UI:
- Provision worker PCs and generate one-time registration UUID tokens.
- Organise workers into custom worker groups for execution targeting.
- Track worker health status (`online`, `busy`, `offline`, `error`), agent versions, screen resolution, and last heartbeat timestamps.
- Rotate worker API keys or deactivate compromised nodes instantly.

### Minimum Worker Agent Version Enforcement
To ensure fleet compatibility and prevent outdated worker software from executing incompatible automation primitives, the task server enforces a configurable minimum agent version requirement (`MIN_AGENT_VERSION`). During periodic heartbeat exchanges (`POST /api/v1/workers/heartbeat`), the server compares the worker's reported `agent_version` against the configured minimum required semantic version. If the worker's agent version is missing or falls below the minimum required version, the server rejects the heartbeat request with an `HTTP 426 Upgrade Required` status code and an error message detailing the version mismatch. This mechanism alerts worker processes to halt polling and initiate an agent update before claiming further automation assignments.

### Execution & Concurrent Dispatch Engine
Manual runs triggered via `POST /automations/{id}/run-now` queue a new entry in `task_runs`. Worker PCs polling `GET /api/v1/workers/next-assignment` execute an atomic transaction using `SELECT ... FOR UPDATE SKIP LOCKED` to claim queued runs matching their assigned worker group. The complete automation hierarchy (steps, details, variables, parameters) is serialized as JSON in the dispatch response.

### Stalled Execution Sweeper & Timeout Recovery
The server background scheduler runs a periodic task every 30 seconds (`sweep_stalled_task_runs`) that checks for active task runs assigned to disconnected worker machines. If a worker assigned to a `running` task run fails to submit a heartbeat within 90 seconds, the task server automatically transitions the task run to `lost` status with a recorded error message. This background recovery loop guarantees that task runs do not remain stuck in an active state indefinitely if a worker PC experiences a power failure, network crash, or agent process exit.

### Automated Data Retention & Storage Maintenance
To prevent database bloat and storage resource exhaustion, the task server executes an hourly background maintenance service that enforces configurable data retention policies. The service automatically prunes expired user sessions (`SESSION_RETENTION_DAYS`, default 7 days), historical audit log entries (`AUDIT_LOG_RETENTION_DAYS`, default 90 days), and completed task run step logs (`TASK_RUN_STEP_RETENTION_DAYS`, default 30 days). For binary assets, step execution screenshots older than `SCREENSHOT_RETENTION_DAYS` (default 30 days) are deleted from the database before removing their corresponding S3 storage objects. Furthermore, the sweeper cleans up abandoned temporary uploads in S3 (`tmp/` prefix older than 24 hours) and performs orphan garbage collection to remove unreferenced S3 objects from `bitmaps/` and `screenshots/` storage paths.

### Runs & Execution History UI
Filterable task execution history interface (`GET /runs`) with support for filtering by automation, worker, status, and date. Step-by-step detail view (`GET /runs/{id}`) utilizing the three-panel CSS magnifier component to display step execution status, captured runtime variables, and runtime screenshots. Editors/admins can request run cancellation (`POST /runs/{id}/cancel`), which flags the run and notifies the executing worker machine upon its next heartbeat.

### Worker Screenshots & Execution Capture
Step execution screenshots captured by worker nodes are uploaded directly to object storage via presigned S3 PUT URLs obtained from `GET /api/v1/workers/task-runs/{id}/screenshot-upload-url`. Worker nodes finalize uploaded screenshots via `POST /api/v1/workers/task-runs/{id}/screenshots/{object_key}/commit`, recording dimensions and linking screenshots to `task_run_steps`.

### Recording Session Capture & Conversion Engine
Remote desktop action recording workflows orchestrated through worker machines:
- Workers poll next assignments to receive `StartRecording` instructions.
- Event streams are posted in batches (`POST /api/v1/workers/recordings/{session_id}/events`).
- Live recording sessions can be monitored via `GET /recordings/{id}` (auto-refreshed via `<meta http-equiv="refresh">`).
- Event review UI (`GET /recordings/{id}/review`) features step cards with exclusion checkboxes and `ImageMagnifier` previews.
- Conversion engine (`POST /recordings/{id}/convert`) bulk-generates draft automations, steps, and reference screenshots from recorded event sequences.

### Scheduling & Cron Engine
Automated schedule management interface (`GET/POST /schedules`, `GET /schedules/new`, `GET/POST /schedules/{id}/edit`, `POST /schedules/{id}/toggle`, `POST /schedules/{id}/delete`). Features server-side 5-field cron expression validation using `croner`. A background task ticks every 30 seconds (`process_due_schedules`), evaluating due schedules, queuing task runs targeted to specified worker groups, and computing next run timestamps.

### User Management & Access Control
Multi-user system supporting roles (`admin`, `editor`, `viewer`), HTTP-only session management, Argon2id password hashing, self-service password updates (`GET/POST /account/password`), and CSRF token validation across all mutation routes.

### Binary CLI Subcommands
In addition to running the web server, the `deskdispatch` application binary supports CLI subcommands (`migrate`, `create-admin`, `init-s3`, `setup`) for container initialization and automated administrative tasks. Running `deskdispatch migrate` executes embedded database migrations, `deskdispatch create-admin` provisions or updates the default admin account using environment variables (`ADMIN_USERNAME`, `ADMIN_PASSWORD`, `ADMIN_DISPLAY_NAME`), and `deskdispatch init-s3` ensures the target S3 bucket is created. The composite `deskdispatch setup` subcommand executes all three initialization steps sequentially, providing a single entrypoint for container startup routines and CI/CD pipelines.

---

## Security & Access Control

DeskDispatch implements strict multi-layer security controls:

1. **Role-Based Access Control (RBAC):**
   - **`viewer`:** Read-only access to dashboard, automations, schedules, runs, and workers.
   - **`editor`:** Create, edit, and delete automations, steps, variables, parameters, bitmaps, schedules, and manual task runs.
   - **`admin`:** Full system permissions including worker management, key rotation, worker groups, and user administration.
2. **Session Security:** Opaque random UUID session keys delivered via `HttpOnly`, `SameSite=Lax` cookies. Active sessions are validated against PostgreSQL and can be revoked server-side immediately.
3. **CSRF Protection:** Server-generated per-session CSRF tokens injected into Askama forms via macros and verified on every non-GET HTTP request.
4. **Rate Limiting:** IP- and username-based login rate limiting to mitigate brute-force credential attacks.
5. **Worker API Security:** Machine API keys are high-entropy 256-bit random tokens hashed with SHA-256 at rest in `task_worker_pcs.api_key_hash`.

---

## Implementation Roadmap & Plan Status

Below is the complete implementation status tracked against `implementation_plan.md`:

### Phase 1: Project Scaffolding & Infrastructure
- [x] Cargo binary crate initialization with Axum, Tokio, SQLx, Askama, S3 SDK
- [x] Docker Compose setup for PostgreSQL and RustFS
- [x] Environment configuration parser (`src/config.rs`)
- [x] `GET /healthz` endpoint with live database connection probe
- [x] Project README documentation

### Phase 2: Database Schema & Migrations
- [x] Migrations for `users`, `sessions`, `audit_log`, `automations`
- [x] Migrations for `automation_variables`, `automation_parameters`, `bitmaps`
- [x] Migrations for `automation_steps` and detail tables (`mouse_clicks`, `key_presses`, `find_pixel_rgb`, `find_bitmap`, `branches`)
- [x] Migrations for `task_worker_pcs`, `worker_groups`, `schedules`, `task_runs`, `recording_sessions`
- [x] Database `CHECK` constraints for discriminated unions and literal-vs-reference pairs
- [x] Database indexes (`idx_steps_automation_position`, `idx_schedules_due`, `idx_task_runs_queued`)
- [x] Seed binary (`src/bin/seed.rs`) for initial admin user creation

### Phase 3: Authentication, Security & RBAC
- [x] `POST /login` and `POST /logout` handlers with Argon2id password verification
- [x] Session management with HTTP-only cookies
- [x] `AuthUser` extractor and RBAC permission checks (`can_edit()`, `can_admin()`)
- [x] CSRF middleware and Askama template form macro injection
- [x] Login rate limiting middleware (`LoginRateLimiter`)
- [x] Self-service password update (`/account/password`)

### Phase 4: Web Application Shell
- [x] Base Askama `layout.html` template and responsive CSS (`static/style.css`)
- [x] Top navigation bar with role-conditional rendering
- [x] Custom 404 and 500 error pages
- [x] Static asset route using `rust-embed`
- [x] Landing Dashboard (`GET /`) with summary stat cards, recent runs, and recent workers

### Phase 5: Automations CRUD & Ordering Logic
- [x] Automations list view (`GET /automations`) with status filter
- [x] Creation form and handler for new automations
- [x] Automation detail view, renaming, status updates, and archive/delete flows
- [x] Step card rendering ordered by sparse float `position`
- [x] Step primitive insertion, move-up, move-down, deletion, and gap compaction

### Phase 6: Object Storage & Zoom Component
- [x] Storage service (`src/storage.rs`) wrapping AWS S3 / RustFS
- [x] Presigned GET and PUT URL generation and presigned POST policy generation
- [x] Reusable Askama three-panel CSS image magnifier component
- [x] Media redirection routes (`/media/screenshots/{id}`, `/media/bitmaps/{id}`)
- [x] Throwaway verification route (`/dev/magnifier-verify`)

### Phase 7: Bitmap Library & Region Picker
- [x] Reference bitmap library list view (`GET /bitmaps`, `GET /automations/{id}/bitmaps`)
- [x] Direct-to-S3 bitmap upload using presigned POST policies
- [x] Bitmap deletion handlers with S3 object removal
- [x] Zero-JS two-click region picker (`top-left`, `bottom-right`, `confirm`)

### Phase 8: Point-Based Step Types & Pickers
- [x] Coordinate translation math mapping clicks to native pixel coordinates (`src/picker.rs`)
- [x] Coarse and fine grid coordinate picking flows
- [x] `mouse_click` step creation and edit forms
- [x] `find_pixel_rgb` step creation and edit forms
- [x] `+ New Step` primitive type picker
- [x] Plain-language string summary generators for steps

### Phase 9: Complex Step Types
- [x] `find_bitmap` step creation and edit forms with reference bitmap selection
- [x] Ad-hoc search region bounding for bitmap search
- [x] `branch` step creation and edit forms with pixel RGB vs bitmap condition toggle
- [x] Dynamic GOTO target step selection based on stable step IDs
- [x] Plain-language string generators for branch conditions

### Phase 10: Variables & Dynamic Binding
- [x] Automation variables CRUD UI and backend handlers
- [x] Automation parameters CRUD UI with default value validation
- [x] Literal-vs-reference UI toggles and form-level validation (`validate_literal_or_reference`)
- [x] Output variable binding for `find_pixel_rgb` and `find_bitmap`
- [x] Dynamic variable name rendering in step summary cards

### Phase 11: Worker API Foundations
- [x] Administrative CRUD UI for `task_worker_pcs` and `worker_groups`
- [x] One-time registration UUID token generation
- [x] Worker deactivation and key rotation handlers
- [x] Worker registration API (`POST /api/v1/workers/register`)
- [x] `AuthWorker` extractor with SHA-256 API key authentication
- [x] Worker heartbeat API (`POST /api/v1/workers/heartbeat`)
- [x] Next assignment polling API (`GET /api/v1/workers/next-assignment`)

### Phase 12: Execution Engine & Dispatch
- [x] `POST /automations/{id}/run-now` manual execution trigger
- [x] Concurrent dispatch claim logic via `SELECT ... FOR UPDATE SKIP LOCKED`
- [x] Full automation JSON payload serialization for worker execution
- [x] Worker run recovery API (`GET /api/v1/workers/task-runs/{id}`)
- [x] Step result submission (`POST /api/v1/workers/task-runs/{id}/step-result`)
- [x] Run completion submission (`POST /api/v1/workers/task-runs/{id}/complete`)
- [x] Worker screenshot upload URL and commit endpoints (`/screenshots/{key}/commit`)
- [x] Background sweeper task for lost task runs
- [x] Filterable runs list UI (`GET /runs`)
- [x] Step-by-step execution detail view (`GET /runs/{id}`)
- [x] Cancel active run handler (`POST /runs/{id}/cancel`)

### Phase 13: Recording Capture & Conversion
- [x] Dispatch `start_recording` instruction to worker PCs
- [x] Ingest batched event streams (`POST /recordings/{id}/events`)
- [x] Live recording status web view with `<meta http-equiv="refresh">`
- [x] Recording review UI with per-event exclusion checkboxes
- [x] Conversion engine turning captured recordings into automations and steps

### Phase 14: Scheduling Engine
- [x] Schedules list view UI (`GET /schedules`)
- [x] Schedule creation/edit forms with 5-field cron validation via `croner`
- [x] Background scheduler tick interval task (`tokio::time::interval`)
- [x] Scheduled database sweep queuing tasks and updating `next_run_at`
- [x] Worker group targeting evaluation during scheduled dispatch

### Phase 15: Hardening, Observability & Deployment
- [x] Global audit log middleware across all mutation handlers
- [x] Database query pass verifying index usage
- [x] Tracing span wrapping across database and storage calls
- [x] Multi-stage production `Dockerfile`
- [x] Production `docker-compose.yml` configuration
- [x] Integration tests for step positioning/compaction and concurrent dispatch claiming
- [x] Final QA pass validating all web operator UI routes, worker API endpoints, form submissions, and execution flows
