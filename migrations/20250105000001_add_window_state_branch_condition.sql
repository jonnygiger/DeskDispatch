-- Migration to add window_state condition to step_branches

ALTER TABLE step_branches DROP CONSTRAINT IF EXISTS step_branches_condition_type_check;

ALTER TABLE step_branches
    ADD CONSTRAINT step_branches_condition_type_check
    CHECK (condition_type IN ('pixel_rgb', 'bitmap', 'window_state'));

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'step_branches' AND column_name = 'window_match_pattern'
    ) THEN
        ALTER TABLE step_branches ADD COLUMN window_match_pattern TEXT;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'step_branches' AND column_name = 'window_match_type'
    ) THEN
        ALTER TABLE step_branches ADD COLUMN window_match_type TEXT DEFAULT 'contains' CHECK (window_match_type IS NULL OR window_match_type IN ('contains', 'exact', 'starts_with'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'step_branches' AND column_name = 'window_check_type'
    ) THEN
        ALTER TABLE step_branches ADD COLUMN window_check_type TEXT DEFAULT 'exists' CHECK (window_check_type IS NULL OR window_check_type IN ('active', 'exists'));
    END IF;
END $$;
