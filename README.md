# DeskDispatch — GUI Automation Task Server

DeskDispatch is a high-performance, zero-JavaScript task server for managing and orchestrating GUI automations across headless task-worker PCs. Built with Rust, Axum, SQLx, PostgreSQL, Askama, and S3-compatible object storage (RustFS), DeskDispatch provides an operator web interface and a lightweight HTTP/JSON polling API for worker machines.

---

## Table of Contents

- [Overview & Architecture](#overview--architecture)
- [C4 Model Architecture Diagram](#c4-model-architecture-diagram)
- [UML Sequence Diagrams](#uml-sequence-diagrams)
  - [1. Worker Task Assignment, Execution, and Screenshot Upload](#1-worker-task-assignment-execution-and-screenshot-upload)
  - [2. Desktop Recording Capture and Automation Conversion](#2-desktop-recording-capture-and-automation-conversion)
  - [3. Scheduled Cron Execution & Stalled Sweeper Failsafe](#3-scheduled-cron-execution--stalled-sweeper-failsafe)
- [Technology Stack](#technology-stack)
- [Zero-JavaScript Interface Design](#zero-javascript-interface-design)
- [Docker Management & Automated Operations](#docker-management--automated-operations)
  - [Prerequisites](#prerequisites)
  - [Starting the Application](#starting-the-application)
  - [Monitoring & Checking Status](#monitoring--checking-status)
  - [Viewing Logs](#viewing-logs)
  - [Stopping the Application](#stopping-the-application)
  - [Resetting Data & Volume Storage](#resetting-data--volume-storage)
  - [Failsafe & Healthcheck Architecture](#failsafe--healthcheck-architecture)
- [Prerequisites & Environment Configuration](#prerequisites--environment-configuration)
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

---

## Overview & Architecture

DeskDispatch serves as the central control plane for GUI automation workflows. Operator users define automations containing sequences of mouse clicks, keystrokes, pixel color checks, reference bitmap searches, and conditional branches. Headless worker PCs poll the task server over HTTPS/JSON to receive assignments, execute steps locally, and report execution results.

```mermaid
graph TD
    Browser["Operator Browser<br/>(Zero-JS HTML/CSS)"]
    Server["DeskDispatch Task Server<br/>(Rust / Axum / Askama)"]
    DB[("PostgreSQL<br/>(Metadata, Steps, Runs, Workers)")]
    Storage[("RustFS / S3 Storage<br/>(Screenshots & Ref Bitmaps)")]
    Workers["Task Worker PCs Fleet<br/>(Headless Agents)"]

    Browser -- "HTTPS / Form POSTs" --> Server
    Server -- "Metadata & Task Queues" --> DB
    Server -- "Presigned Object URLs" --> Storage
    Workers -- "HTTPS / JSON Poll & Event Batches" --> Server
    Workers -- "Direct Presigned Uploads/Downloads" --> Storage
```

**Key Architectural Guarantees:**
- **Worker-Initiated Pull Model:** Workers poll the server via long-polling (`GET /api/v1/workers/next-assignment`). The server never opens inbound connections to worker PCs, enabling deployment across NATs and firewalls.
- **Zero-JavaScript Web Interface:** All web views are rendered on the server using Askama templates. Form submissions use standard HTTP POST with 303 Redirects. Interaction mechanisms like coordinate pickers leverage native HTML `<input type="image">`.
- **Atomic Dispatch:** Task run claiming utilizes PostgreSQL `SELECT ... FOR UPDATE SKIP LOCKED` queries to guarantee thread-safe, concurrent task distribution across multiple worker nodes without external locking services.

---

## C4 Model Architecture Diagram

The C4 Model Container diagram illustrates the high-level software architecture, boundaries, and interactions among system components:

```mermaid
C4Container
    title Container Diagram for DeskDispatch Task Server

    Person(operator, "Operator / Admin", "Manages automations, schedules, worker fleets, and reviews execution runs.")
    Person_Ext(worker_agent, "Task Worker Agent", "Headless worker process executing GUI actions (clicks, keystrokes, pixel/bitmap searches).")

    System_Boundary(deskdispatch_sys, "DeskDispatch Automation Control Plane") {
        Container(web_app, "DeskDispatch Web & API Server", "Rust / Axum / Askama", "Serves Zero-JS HTML operator UI, background schedulers, sweeper tasks, and HTTP JSON Worker API endpoints.")
        ContainerDb(database, "PostgreSQL Database", "PostgreSQL 16", "Stores user credentials, audit logs, automation definitions, steps, schedules, worker metadata, and task run states.")
        ContainerDb(object_storage, "RustFS Object Storage", "S3-Compatible Storage", "Stores reference bitmaps, region crops, and execution screenshots.")
        Container(migration_job, "Init / Migration Job", "DeskDispatch CLI (`setup`)", "Runs database migrations, seeds default admin user, and initializes S3 storage buckets.")
    }

    Rel(operator, web_app, "Interacts with UI via HTML forms and CSS magnifier", "HTTPS")
    Rel(worker_agent, web_app, "Polls assignments, sends heartbeats, submits step results and recording events", "HTTPS / JSON")
    Rel(worker_agent, object_storage, "Uploads execution screenshots & downloads reference bitmaps", "HTTPS / S3 Presigned URLs")
    Rel(web_app, database, "Reads & writes metadata, locks queue rows via `FOR UPDATE SKIP LOCKED`", "SQLx / TCP")
    Rel(web_app, object_storage, "Generates presigned GET/PUT URLs and manages retention cleanup", "AWS SigV4 / HTTP")
    Rel(migration_job, database, "Applies SQL migrations and seeds admin account", "SQLx / TCP")
    Rel(migration_job, object_storage, "Ensures bucket existence and lifecycle rules", "S3 API")
```

---

## UML Sequence Diagrams

### 1. Worker Task Assignment, Execution, and Screenshot Upload

This sequence details how manual or scheduled tasks are claimed atomically by task worker machines, executed step-by-step, and logged with presigned screenshot uploads:

```mermaid
sequenceDiagram
    autonumber
    actor Operator as Operator / Admin
    participant Server as DeskDispatch Server
    participant DB as PostgreSQL DB
    participant S3 as RustFS S3 Storage
    participant Worker as Task Worker Machine

    Operator->>Server: POST /automations/{id}/run-now
    Server->>DB: INSERT into task_runs (status = 'queued')
    Server-->>Operator: 303 Redirect to /runs/{run_id}

    loop Long-Poll Assignment Loop
        Worker->>Server: GET /api/v1/workers/next-assignment
        Server->>DB: SELECT ... FOR UPDATE SKIP LOCKED
        DB-->>Server: Claimed task_run details
        Server-->>Worker: 200 OK (Task assignment + Full Automation JSON)
    end

    loop Step Execution
        Worker->>Worker: Execute GUI Step (Click / Keypress / Bitmap Search)
        Worker->>Server: GET /api/v1/workers/task-runs/{id}/screenshot-upload-url
        Server-->>Worker: 200 OK (Presigned S3 PUT URL)
        Worker->>S3: PUT /screenshots/{key}.png (Image Data)
        Worker->>Server: POST /api/v1/workers/task-runs/{id}/screenshots/{key}/commit
        Worker->>Server: POST /api/v1/workers/task-runs/{id}/step-result
        Server->>DB: UPDATE task_run_steps & variable_values
    end

    Worker->>Server: POST /api/v1/workers/task-runs/{id}/complete (status = 'succeeded')
    Server->>DB: UPDATE task_runs SET status = 'succeeded'
```

### 2. Desktop Recording Capture and Automation Conversion

This sequence demonstrates live user action recording on worker machines and draft conversion into executable automations:

```mermaid
sequenceDiagram
    autonumber
    actor Operator as Operator / Admin
    participant Server as DeskDispatch Server
    participant DB as PostgreSQL DB
    participant S3 as RustFS S3 Storage
    participant Worker as Task Worker Machine

    Operator->>Server: POST /workers/{id}/record/start
    Server->>DB: INSERT into recording_sessions (status = 'recording')

    loop Worker Heartbeat & Poll
        Worker->>Server: POST /api/v1/workers/heartbeat
        Server-->>Worker: Response with active recording session ID
    end

    loop Capture Stream
        Worker->>Worker: Capture Mouse/Keyboard OS Events
        Worker->>Server: GET /api/v1/workers/recordings/{session_id}/screenshot-upload-url
        Server-->>Worker: Presigned S3 PUT URL
        Worker->>S3: PUT recording event screenshot
        Worker->>Server: POST /api/v1/workers/recordings/{session_id}/events
        Server->>DB: INSERT into recording_events
    end

    Operator->>Server: POST /recordings/{id}/stop
    Server->>DB: UPDATE recording_sessions SET status = 'completed'
    Operator->>Server: GET /recordings/{id}/review
    Operator->>Server: POST /recordings/{id}/convert (Select events)
    Server->>DB: Create automation, automation_steps & details
    Server-->>Operator: 303 Redirect to /automations/{new_id}
```

### 3. Scheduled Cron Execution & Stalled Sweeper Failsafe

This sequence highlights background cron schedule processing alongside automated failsafe recovery for stalled task runs:

```mermaid
sequenceDiagram
    autonumber
    participant Cron as Cron Scheduler Loop
    participant Sweeper as Stalled Execution Sweeper
    participant Server as DeskDispatch Server
    participant DB as PostgreSQL DB

    rect rgb(240, 248, 255)
        note over Cron, DB: Scheduled Execution Trigger
        Cron->>DB: Query due schedules (process_due_schedules)
        Cron->>DB: INSERT task_runs (status = 'queued', target_worker_group_id)
        Cron->>DB: UPDATE schedules SET next_run_at
    end

    rect rgb(255, 240, 240)
        note over Sweeper, DB: Stalled Task Run Eviction (Failsafe)
        Sweeper->>DB: Query running task_runs where worker heartbeat > 90s
        Sweeper->>DB: UPDATE task_runs SET status = 'lost', error_message
    end
```

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

## Docker Management & Automated Operations

DeskDispatch is fully containerized and configured for automatic startup, lifecycle management, and failsafe execution through Docker Compose.

### Prerequisites
- **Docker Engine:** v20.10+
- **Docker Compose:** v2.0+

### Starting the Application

To start the entire application stack (PostgreSQL, RustFS Object Storage, Database Migration & Seeding Job, and the DeskDispatch Task Server) in background daemon mode:

```bash
docker compose up -d
```

**Automatic Startup Sequence:**
1. **PostgreSQL Container (`deskdispatch-postgres`):** Starts and waits for healthy database readiness (`pg_isready`).
2. **RustFS Storage Container (`deskdispatch-rustfs`):** Starts and waits for healthy S3 API readiness (`/health`).
3. **Migration & Initialization Job (`deskdispatch-migration`):** Runs `/app/deskdispatch setup` to execute database migrations, seed the initial administrator account (`admin` / `test`), and create the S3 bucket (`deskdispatch-bucket`).
4. **DeskDispatch App Container (`deskdispatch-app`):** Boots after `migration` completes successfully and begins serving operator requests on port 3000.

### Monitoring & Checking Status

Inspect running containers and health statuses:

```bash
docker compose ps
```

Probe application health endpoints:

```bash
# Basic liveness check
curl http://localhost:3000/livez

# Database & system readiness check
curl http://localhost:3000/readyz
```

### Viewing Logs

Stream live logs across all services:

```bash
docker compose logs -f
```

Stream logs for a specific service (e.g., the web server):

```bash
docker compose logs -f app
```

### Stopping the Application

To stop all running containers while preserving database and object storage volumes:

```bash
docker compose down
```

### Resetting Data & Volume Storage

To stop all containers and clear persistent storage volumes (e.g. for a clean re-initialization):

```bash
docker compose down -v
```

### Failsafe & Healthcheck Architecture

The Docker setup incorporates multi-layered health checks and fail-fast guarantees:
- **Dependency Chains (`depends_on`):** Service dependencies use explicit `condition: service_healthy` and `condition: service_completed_successfully` clauses to eliminate race conditions during boot.
- **Database Readiness (`pg_isready`):** `deskdispatch-postgres` performs health checks every 5 seconds to ensure PostgreSQL accepts connection pools before application startup.
- **Storage Readiness (`curl`):** `deskdispatch-rustfs` validates HTTP storage layer health before migrations or application startup.
- **Application Liveness (`deskdispatch healthcheck`):** `deskdispatch-app` uses its internal binary healthcheck subcommand to verify TCP and HTTP `/livez` responsiveness every 10 seconds.
- **Graceful Shutdown & Signal Handling:** `deskdispatch-app` listens for SIGINT/SIGTERM signals, canceling background tasks and flushing pending database queries before exiting.
- **Production Credential Validation:** In production mode (`APP_ENV=production`), `Config::validate()` fails fast on startup if default or weak credentials are detected for `SESSION_SECRET`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`, or `DATABASE_URL`.

---

## Prerequisites & Environment Configuration

### Environment Variables (`.env`)

```env
# Database
DATABASE_URL=postgres://postgres:deskdispatch_db_prod_secret_9823@postgres:5432/deskdispatch

# Object Storage (S3 / RustFS)
S3_ENDPOINT=http://rustfs:9000
S3_PUBLIC_ENDPOINT=http://localhost:9000
S3_BUCKET=deskdispatch-bucket
S3_ACCESS_KEY=deskdispatch_s3_user
S3_SECRET_KEY=deskdispatch_s3_prod_secret_4712
S3_REGION=us-east-1

# Application Security & Server Config
SESSION_SECRET=deskdispatch_session_prod_secret_8832
BIND_ADDRESS=0.0.0.0:3000
APP_ENV=production
MIN_AGENT_VERSION=1.0.0
```

### Public S3 Endpoint URL Rewriting (`S3_PUBLIC_ENDPOINT`)
When DeskDispatch runs inside a Docker network or behind a private infrastructure network, `S3_ENDPOINT` typically points to an internal container address (e.g., `http://rustfs:9000`). However, operator browsers and external task-worker PCs need to access presigned GET/PUT URLs and POST policies using a publicly reachable host. By setting `S3_PUBLIC_ENDPOINT` (e.g., `http://localhost:9000`), the task server automatically rewrites generated presigned object storage URLs from the internal endpoint to the public endpoint, allowing external clients to upload screenshots and download reference bitmaps directly without exposing internal network routing.

---

## Web Operator Interface — Route Map

All GET routes render Askama HTML templates; all POST routes mutate state and issue HTTP 303 redirects.

| Area | Route Path | Method | Minimum Role | Status | Description |
| --- | --- | --- | --- | --- | --- |
| **System** | `/healthz` | GET | Public | Implemented | Health check endpoint |
| | `/livez` | GET | Public | Implemented | Liveness probe endpoint |
| | `/readyz` | GET | Public | Implemented | System & database readiness probe |
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

```mermaid
erDiagram
    users ||--o{ sessions : "creates"
    users ||--o{ audit_log : "performs"
    automations ||--o{ automation_steps : "contains"
    automations ||--o{ automation_variables : "defines"
    automations ||--o{ automation_parameters : "defines"
    automations ||--o{ task_runs : "instantiates"
    automation_steps ||--o| step_mouse_clicks : "discriminates"
    automation_steps ||--o| step_key_presses : "discriminates"
    automation_steps ||--o| step_find_pixel_rgb : "discriminates"
    automation_steps ||--o| step_find_bitmap : "discriminates"
    automation_steps ||--o| step_branches : "discriminates"
    task_worker_pcs }|--|{ worker_group_members : "belongs to"
    worker_groups ||--o{ worker_group_members : "groups"
    worker_groups ||--o{ schedules : "targets"
    automations ||--o{ schedules : "scheduled by"
    task_runs ||--o{ task_run_steps : "logs"
    task_runs ||--o{ task_run_variable_values : "captures"
    recording_sessions ||--o{ recording_events : "streams"

    automations {
        uuid id PK
        string name
        string status
    }
    automation_steps {
        uuid id PK
        uuid automation_id FK
        float position
        string step_type
        int post_delay_ms
    }
    task_runs {
        uuid id PK
        uuid automation_id FK
        uuid worker_id FK
        string status
        timestamp queued_at
    }
    task_worker_pcs {
        uuid id PK
        string hostname
        string status
        timestamp last_heartbeat_at
    }
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
6. **Reverse Proxy Header Trust (`TRUST_PROXY_HEADERS`):** When deployed behind a reverse proxy or load balancer (e.g., NGINX or AWS ALB), setting `TRUST_PROXY_HEADERS=true` enables parsing of `X-Forwarded-For` and `X-Real-IP` HTTP request headers to identify actual client IP addresses for login rate limiting and session audit logs. By default (`false`), proxy headers are ignored to prevent IP spoofing attacks when the application receives direct client connections. This mechanism ensures accurate rate-limiting enforcement and security logging across containerized and cloud proxy deployments.
