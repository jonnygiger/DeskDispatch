-- Add sequence number column and unique constraint for worker step result idempotency
ALTER TABLE task_run_steps
    ADD COLUMN IF NOT EXISTS seq INTEGER;

CREATE UNIQUE INDEX IF NOT EXISTS idx_task_run_steps_run_seq
    ON task_run_steps (task_run_id, seq);
