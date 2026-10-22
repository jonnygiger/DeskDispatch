-- Migration to add mouse_scroll step primitive support

ALTER TABLE automation_steps DROP CONSTRAINT IF EXISTS automation_steps_step_type_check;

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_step_type_check
    CHECK (step_type IN ('mouse_click', 'key_press', 'type_text', 'find_pixel_rgb', 'find_bitmap', 'branch', 'window_control', 'ocr_extract', 'mouse_scroll'));

CREATE TABLE step_mouse_scrolls (
    step_id            BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    direction          TEXT NOT NULL DEFAULT 'down' CHECK (direction IN ('up', 'down', 'left', 'right')),
    scroll_amount      INTEGER NOT NULL DEFAULT 3 CHECK (scroll_amount >= 1 AND scroll_amount <= 100),
    x                  INTEGER CHECK (x IS NULL OR x >= 0),
    y                  INTEGER CHECK (y IS NULL OR y >= 0),
    x_variable_id      BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    y_variable_id      BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    region_variable_id BIGINT REFERENCES automation_variables(id) ON DELETE SET NULL,
    CHECK (x IS NULL OR x_variable_id IS NULL),
    CHECK (y IS NULL OR y_variable_id IS NULL)
);
