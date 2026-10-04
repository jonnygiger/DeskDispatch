# Scribe's Journal

## 2025-05-20 - Stalled Execution Sweeper & Timeout Recovery
Learning: The background task sweeper (`sweep_stalled_task_runs`) was fully functional and scheduled every 30s in `main.rs`, but was not documented in `README.md`.
Action: Added clear explanation under `## Features & System Capabilities` detailing the 90s heartbeat threshold and transition to `lost` status for abandoned task runs.
