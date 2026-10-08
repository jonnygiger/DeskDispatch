-- Add parameter_overrides JSONB column to task_runs
ALTER TABLE task_runs
    ADD COLUMN IF NOT EXISTS parameter_overrides JSONB;
