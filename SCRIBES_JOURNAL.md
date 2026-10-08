# Scribe's Journal

## 2025-05-20 - Stalled Execution Sweeper & Timeout Recovery
Learning: The background task sweeper (`sweep_stalled_task_runs`) was fully functional and scheduled every 30s in `main.rs`, but was not documented in `README.md`.
Action: Added clear explanation under `## Features & System Capabilities` detailing the 90s heartbeat threshold and transition to `lost` status for abandoned task runs.

## 2025-05-20 - Final QA Pass & Project Documentation Audit
Learning: All web operator interface routes, worker API endpoints, background tasks, and implementation roadmap items across Phases 12–15 have been fully implemented and verified in code and tests. `README.md` contained lingering `[TODO]` markers from earlier roadmap drafts.
Action: Updated `README.md` Table of Contents, Route Map, Worker API Reference, Features & System Capabilities, and Implementation Roadmap to accurately reflect 100% completion status.

## 2025-05-20 - Background Data Retention & Storage Cleanup Configuration
Learning: Background maintenance jobs (session pruning, audit log pruning, step log pruning, screenshot cleanup, abandoned S3 upload cleanup, and orphan S3 object GC) in `src/retention.rs` are executed hourly in `main.rs` and configurable via environment variables, but were entirely omitted from the feature documentation.
Action: Documented the automated retention service under `## Features & System Capabilities` with explicit details on default retention windows and S3 storage maintenance routines.

## 2025-05-20 - Reverse Proxy Header Trust Configuration
Learning: The `TRUST_PROXY_HEADERS` environment flag and corresponding `extract_client_ip` proxy header parsing logic (`X-Forwarded-For` / `X-Real-IP`) in `src/routes/auth.rs` was fully implemented but missing from `README.md`.
Action: Documented `TRUST_PROXY_HEADERS` under `## Security & Access Control` explaining client IP extraction, rate limiting context, and anti-spoofing defaults when deployed behind reverse proxies.
