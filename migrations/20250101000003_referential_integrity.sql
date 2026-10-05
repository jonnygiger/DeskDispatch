-- Migration 20250101000003_referential_integrity.sql
-- Fix deletion and referential integrity for automations and steps

-- 1. Add deleted_at to automation_steps for soft deletion when step has run history
ALTER TABLE automation_steps
    ADD COLUMN deleted_at TIMESTAMPTZ;

-- Partial unique constraint so soft-deleted steps don't collide on (automation_id, position)
ALTER TABLE automation_steps
    DROP CONSTRAINT automation_steps_automation_id_position_key;

CREATE UNIQUE INDEX idx_automation_steps_active_position
    ON automation_steps (automation_id, position)
    WHERE deleted_at IS NULL;

-- 2. Update step_branches foreign keys to ON DELETE SET NULL for match/no_match targets
ALTER TABLE step_branches
    DROP CONSTRAINT IF EXISTS step_branches_on_match_step_id_fkey,
    DROP CONSTRAINT IF EXISTS step_branches_on_no_match_step_id_fkey;

ALTER TABLE step_branches
    ADD CONSTRAINT step_branches_on_match_step_id_fkey
        FOREIGN KEY (on_match_step_id) REFERENCES automation_steps(id) ON DELETE SET NULL,
    ADD CONSTRAINT step_branches_on_no_match_step_id_fkey
        FOREIGN KEY (on_no_match_step_id) REFERENCES automation_steps(id) ON DELETE SET NULL;

-- 3. Update task_runs and task_run_variable_values foreign keys to ON DELETE SET NULL
ALTER TABLE task_runs
    DROP CONSTRAINT IF EXISTS task_runs_current_step_id_fkey;

ALTER TABLE task_runs
    ADD CONSTRAINT task_runs_current_step_id_fkey
        FOREIGN KEY (current_step_id) REFERENCES automation_steps(id) ON DELETE SET NULL;

ALTER TABLE task_run_variable_values
    DROP CONSTRAINT IF EXISTS task_run_variable_values_set_at_step_id_fkey;

ALTER TABLE task_run_variable_values
    ADD CONSTRAINT task_run_variable_values_set_at_step_id_fkey
        FOREIGN KEY (set_at_step_id) REFERENCES automation_steps(id) ON DELETE SET NULL;
