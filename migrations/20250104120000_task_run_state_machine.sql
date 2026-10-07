-- Migration 20250101000004_task_run_state_machine.sql
-- Task run state machine enhancements: cancelling state, cancel_requested_at, and single active run constraint per worker.

-- 1. Add cancel_requested_at column to task_runs
ALTER TABLE task_runs
    ADD COLUMN IF NOT EXISTS cancel_requested_at TIMESTAMPTZ;

-- 2. Update status CHECK constraint to include 'cancelling'
ALTER TABLE task_runs
    DROP CONSTRAINT IF EXISTS task_runs_status_check;

ALTER TABLE task_runs
    ADD CONSTRAINT task_runs_status_check
    CHECK (status IN ('queued', 'running', 'cancelling', 'succeeded', 'failed', 'cancelled', 'lost'));

-- 3. Add partial unique index so a worker cannot claim a second task run while executing an active run
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_runs_worker_running
    ON task_runs (worker_id)
    WHERE status IN ('running', 'cancelling');
