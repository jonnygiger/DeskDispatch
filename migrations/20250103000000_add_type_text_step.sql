-- Migration to add type_text step primitive support

ALTER TABLE automation_steps DROP CONSTRAINT IF EXISTS automation_steps_step_type_check;

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_step_type_check
    CHECK (step_type IN ('mouse_click', 'key_press', 'type_text', 'find_pixel_rgb', 'find_bitmap', 'branch'));

CREATE TABLE step_type_texts (
    step_id               BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    text_to_type          TEXT NOT NULL,
    delay_between_keys_ms INTEGER NOT NULL DEFAULT 0 CHECK (delay_between_keys_ms >= 0)
);
