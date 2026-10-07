-- Migration 20250110120000_performance_indexes.sql

-- 1. Replace redundant idx_task_runs_queued index on status, queued_at with queued_at, id partial index
DROP INDEX IF EXISTS idx_task_runs_queued;
CREATE INDEX idx_task_runs_queued ON task_runs (queued_at, id) WHERE status = 'queued';

-- 2. Index on task_run_steps for task run detail and step lookups
CREATE INDEX IF NOT EXISTS idx_task_run_steps_task_run_id ON task_run_steps (task_run_id, id);

-- 3. Indexes on task_runs for automation, worker, and keyset pagination queries
CREATE INDEX IF NOT EXISTS idx_task_runs_automation_worker ON task_runs (automation_id, worker_id);
CREATE INDEX IF NOT EXISTS idx_task_runs_worker_id ON task_runs (worker_id);
CREATE INDEX IF NOT EXISTS idx_task_runs_queued_at_id ON task_runs (queued_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_schedule_id ON task_runs (schedule_id);

-- 4. Index on sessions for user active session lookups and purging
CREATE INDEX IF NOT EXISTS idx_sessions_user_expires ON sessions (user_id, expires_at);

-- 5. Index on audit_log for time-series audit queries
CREATE INDEX IF NOT EXISTS idx_audit_log_created_at ON audit_log (created_at DESC);

-- 6. Index on task_worker_pcs for rotated key lookups
CREATE INDEX IF NOT EXISTS idx_task_worker_pcs_prev_api_key_hash
    ON task_worker_pcs (previous_api_key_hash)
    WHERE previous_api_key_hash IS NOT NULL;

-- 7. Foreign key indexes for step_branches ON DELETE SET NULL cascades
CREATE INDEX IF NOT EXISTS idx_step_branches_on_match_step_id ON step_branches (on_match_step_id);
CREATE INDEX IF NOT EXISTS idx_step_branches_on_no_match_step_id ON step_branches (on_no_match_step_id);
