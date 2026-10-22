-- Migration to add window_state condition to step_branches

ALTER TABLE step_branches DROP CONSTRAINT IF EXISTS step_branches_condition_type_check;

ALTER TABLE step_branches
    ADD CONSTRAINT step_branches_condition_type_check
    CHECK (condition_type IN ('pixel_rgb', 'bitmap', 'window_state'));

ALTER TABLE step_branches
    ADD COLUMN window_match_pattern TEXT,
    ADD COLUMN window_match_type    TEXT DEFAULT 'contains' CHECK (window_match_type IS NULL OR window_match_type IN ('contains', 'exact', 'starts_with')),
    ADD COLUMN window_check_type    TEXT DEFAULT 'exists' CHECK (window_check_type IS NULL OR window_check_type IN ('active', 'exists'));
