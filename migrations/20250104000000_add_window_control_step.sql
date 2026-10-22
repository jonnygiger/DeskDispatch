-- Migration to add window_control step primitive support

ALTER TABLE automation_steps DROP CONSTRAINT IF EXISTS automation_steps_step_type_check;

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_step_type_check
    CHECK (step_type IN ('mouse_click', 'key_press', 'type_text', 'find_pixel_rgb', 'find_bitmap', 'branch', 'window_control'));

CREATE TABLE step_window_controls (
    step_id       BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    action        TEXT NOT NULL CHECK (action IN ('activate', 'close', 'minimize', 'maximize', 'wait')),
    match_pattern TEXT NOT NULL,
    match_type    TEXT NOT NULL DEFAULT 'contains' CHECK (match_type IN ('contains', 'exact', 'starts_with')),
    timeout_ms    INTEGER NOT NULL DEFAULT 5000 CHECK (timeout_ms >= 0)
);
