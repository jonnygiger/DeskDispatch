-- Migration 20250109120000_schema_hardening.sql
-- Schema Hardening: CHECK ranges, NOT NULL targets, composite FKs, lower(username) uniqueness, default session UUID

-- 1. Default UUID for sessions.id
ALTER TABLE sessions
    ALTER COLUMN id SET DEFAULT gen_random_uuid();

-- 2. Case-insensitive unique index for usernames
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_lower_username
    ON users (lower(username));

-- 3. CHECK range constraints for non-negative values, valid RGB (0..255), tolerance, dimensions, match thresholds
ALTER TABLE automation_steps
    ADD CONSTRAINT chk_automation_steps_post_delay_ms CHECK (post_delay_ms >= 0);

ALTER TABLE step_mouse_clicks
    ADD CONSTRAINT chk_step_mouse_clicks_x CHECK (x IS NULL OR x >= 0),
    ADD CONSTRAINT chk_step_mouse_clicks_y CHECK (y IS NULL OR y >= 0);

ALTER TABLE step_find_pixel_rgb
    ADD CONSTRAINT chk_step_find_pixel_rgb_x CHECK (x >= 0),
    ADD CONSTRAINT chk_step_find_pixel_rgb_y CHECK (y >= 0);

ALTER TABLE step_find_bitmap
    ADD CONSTRAINT chk_step_find_bitmap_search_x CHECK (search_x IS NULL OR search_x >= 0),
    ADD CONSTRAINT chk_step_find_bitmap_search_y CHECK (search_y IS NULL OR search_y >= 0),
    ADD CONSTRAINT chk_step_find_bitmap_search_width CHECK (search_width IS NULL OR search_width > 0),
    ADD CONSTRAINT chk_step_find_bitmap_search_height CHECK (search_height IS NULL OR search_height > 0),
    ADD CONSTRAINT chk_step_find_bitmap_match_threshold CHECK (match_threshold >= 0.0 AND match_threshold <= 1.0);

ALTER TABLE step_branches
    ADD CONSTRAINT chk_step_branches_x CHECK (x IS NULL OR x >= 0),
    ADD CONSTRAINT chk_step_branches_y CHECK (y IS NULL OR y >= 0),
    ADD CONSTRAINT chk_step_branches_search_x CHECK (search_x IS NULL OR search_x >= 0),
    ADD CONSTRAINT chk_step_branches_search_y CHECK (search_y IS NULL OR search_y >= 0),
    ADD CONSTRAINT chk_step_branches_search_width CHECK (search_width IS NULL OR search_width > 0),
    ADD CONSTRAINT chk_step_branches_search_height CHECK (search_height IS NULL OR search_height > 0),
    ADD CONSTRAINT chk_step_branches_match_threshold CHECK (match_threshold IS NULL OR (match_threshold >= 0.0 AND match_threshold <= 1.0)),
    ADD CONSTRAINT chk_step_branches_expected_r CHECK (expected_r IS NULL OR (expected_r >= 0 AND expected_r <= 255)),
    ADD CONSTRAINT chk_step_branches_expected_g CHECK (expected_g IS NULL OR (expected_g >= 0 AND expected_g <= 255)),
    ADD CONSTRAINT chk_step_branches_expected_b CHECK (expected_b IS NULL OR (expected_b >= 0 AND expected_b <= 255)),
    ADD CONSTRAINT chk_step_branches_tolerance CHECK (tolerance IS NULL OR (tolerance >= 0 AND tolerance <= 255));

ALTER TABLE bitmaps
    ADD CONSTRAINT chk_bitmaps_width CHECK (width > 0),
    ADD CONSTRAINT chk_bitmaps_height CHECK (height > 0);

ALTER TABLE step_screenshots
    ADD CONSTRAINT chk_step_screenshots_width CHECK (width > 0),
    ADD CONSTRAINT chk_step_screenshots_height CHECK (height > 0);

ALTER TABLE task_worker_pcs
    ADD CONSTRAINT chk_task_worker_pcs_screen_width CHECK (screen_width IS NULL OR screen_width > 0),
    ADD CONSTRAINT chk_task_worker_pcs_screen_height CHECK (screen_height IS NULL OR screen_height > 0);

ALTER TABLE recording_events
    ADD CONSTRAINT chk_recording_events_x CHECK (x IS NULL OR x >= 0),
    ADD CONSTRAINT chk_recording_events_y CHECK (y IS NULL OR y >= 0);

ALTER TABLE task_run_steps
    ADD CONSTRAINT chk_task_run_steps_captured_r CHECK (captured_r IS NULL OR (captured_r >= 0 AND captured_r <= 255)),
    ADD CONSTRAINT chk_task_run_steps_captured_g CHECK (captured_g IS NULL OR (captured_g >= 0 AND captured_g <= 255)),
    ADD CONSTRAINT chk_task_run_steps_captured_b CHECK (captured_b IS NULL OR (captured_b >= 0 AND captured_b <= 255)),
    ADD CONSTRAINT chk_task_run_steps_captured_x CHECK (captured_x IS NULL OR captured_x >= 0),
    ADD CONSTRAINT chk_task_run_steps_captured_y CHECK (captured_y IS NULL OR captured_y >= 0);

-- 4. Composite unique constraint on automation_steps (automation_id, id) & step_branches automation isolation
CREATE UNIQUE INDEX IF NOT EXISTS idx_automation_steps_automation_id_id
    ON automation_steps (automation_id, id);

ALTER TABLE step_branches
    ADD COLUMN IF NOT EXISTS automation_id BIGINT;

UPDATE step_branches sb
SET automation_id = ast.automation_id
FROM automation_steps ast
WHERE sb.step_id = ast.id AND sb.automation_id IS NULL;

ALTER TABLE step_branches
    ALTER COLUMN automation_id SET NOT NULL;

-- Remove old foreign keys on step_branches if they exist
ALTER TABLE step_branches
    DROP CONSTRAINT IF EXISTS step_branches_on_match_step_id_fkey,
    DROP CONSTRAINT IF EXISTS step_branches_on_no_match_step_id_fkey;

-- Enforce NOT NULL on branch targets
ALTER TABLE step_branches
    ALTER COLUMN on_match_step_id SET NOT NULL,
    ALTER COLUMN on_no_match_step_id SET NOT NULL;

-- Add composite foreign keys to ensure branch targets belong to the same automation
ALTER TABLE step_branches
    ADD CONSTRAINT step_branches_step_automation_fkey
        FOREIGN KEY (step_id, automation_id)
        REFERENCES automation_steps(id, automation_id)
        ON DELETE CASCADE,
    ADD CONSTRAINT step_branches_on_match_fk
        FOREIGN KEY (automation_id, on_match_step_id)
        REFERENCES automation_steps(automation_id, id),
    ADD CONSTRAINT step_branches_on_no_match_fk
        FOREIGN KEY (automation_id, on_no_match_step_id)
        REFERENCES automation_steps(automation_id, id);
