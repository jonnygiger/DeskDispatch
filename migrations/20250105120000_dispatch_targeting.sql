-- Add target_worker_group_id and dispatched_automation_json to task_runs
ALTER TABLE task_runs
    ADD COLUMN IF NOT EXISTS target_worker_group_id BIGINT REFERENCES worker_groups(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS dispatched_automation_json JSONB;

-- Add index on status and target_worker_group_id for efficient dispatch querying
CREATE INDEX IF NOT EXISTS idx_task_runs_target_worker_group
    ON task_runs (target_worker_group_id)
    WHERE status = 'queued';
