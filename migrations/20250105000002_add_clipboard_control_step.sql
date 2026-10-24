-- Migration to add clipboard_control step primitive support

ALTER TABLE automation_steps DROP CONSTRAINT IF EXISTS automation_steps_step_type_check;

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_step_type_check
    CHECK (step_type IN ('mouse_click', 'key_press', 'type_text', 'find_pixel_rgb', 'find_bitmap', 'branch', 'window_control', 'ocr_extract', 'mouse_scroll', 'clipboard_control'));

CREATE TABLE IF NOT EXISTS step_clipboard_controls (
    step_id            BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    action             TEXT NOT NULL DEFAULT 'set_text'
                           CHECK (action IN ('set_text', 'get_text', 'clear', 'wait_for_change')),
    text_to_set        TEXT,
    input_variable_id  BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    output_variable_id BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    set_as_task_output BOOLEAN NOT NULL DEFAULT FALSE,
    timeout_ms         BIGINT NOT NULL DEFAULT 5000 CHECK (timeout_ms >= 0)
);
