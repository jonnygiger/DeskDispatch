-- Add task output support and step_ocr_extracts table

DO $$
BEGIN
    -- 1. Ensure 'ocr_extract' is allowed in automation_steps step_type check constraint
    IF EXISTS (
        SELECT 1 FROM information_schema.check_constraints
        WHERE constraint_name = 'automation_steps_step_type_check'
    ) THEN
        ALTER TABLE automation_steps DROP CONSTRAINT automation_steps_step_type_check;
        ALTER TABLE automation_steps ADD CONSTRAINT automation_steps_step_type_check
            CHECK (step_type IN ('mouse_click', 'key_press', 'type_text', 'find_pixel_rgb', 'find_bitmap', 'branch', 'ocr_extract'));
    END IF;

    -- 2. Add output_type and output_value to task_runs if not present
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'task_runs' AND column_name = 'output_type'
    ) THEN
        ALTER TABLE task_runs ADD COLUMN output_type TEXT CHECK (output_type IN ('boolean', 'image', 'ocr_text'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'task_runs' AND column_name = 'output_value'
    ) THEN
        ALTER TABLE task_runs ADD COLUMN output_value TEXT;
    END IF;

    -- 3. Add expected_output_type to automations if not present
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'automations' AND column_name = 'expected_output_type'
    ) THEN
        ALTER TABLE automations ADD COLUMN expected_output_type TEXT CHECK (expected_output_type IN ('boolean', 'image', 'ocr_text'));
    END IF;
END $$;

-- 4. Create table step_ocr_extracts if not exists
CREATE TABLE IF NOT EXISTS step_ocr_extracts (
    step_id BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    search_x INT,
    search_y INT,
    search_width INT,
    search_height INT,
    region_variable_id BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    output_variable_id BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    set_as_task_output BOOLEAN NOT NULL DEFAULT TRUE,
    timeout_ms BIGINT NOT NULL DEFAULT 5000,
    retry_interval_ms BIGINT NOT NULL DEFAULT 250
);
