-- Migration 20250124000000_schema_alignment.sql
-- Alignment migration: Ensure screenshots table, sessions.last_active_at, and task_runs.parameter_overrides exist.

-- 1. Create screenshots table (or alias view / table compatible with step_screenshots)
CREATE TABLE IF NOT EXISTS screenshots (
    id                 BIGSERIAL PRIMARY KEY,
    step_id            BIGINT REFERENCES automation_steps(id) ON DELETE CASCADE,
    object_storage_key TEXT NOT NULL,
    width              INTEGER NOT NULL CHECK (width > 0),
    height             INTEGER NOT NULL CHECK (height > 0),
    captured_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_screenshots_step_id ON screenshots(step_id);

-- 2. Ensure sessions table has last_active_at column
ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS last_active_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- 3. Ensure task_runs table has parameter_overrides column
ALTER TABLE task_runs
    ADD COLUMN IF NOT EXISTS parameter_overrides JSONB;
