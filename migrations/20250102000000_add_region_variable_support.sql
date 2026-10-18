-- Add region variable support and step detail table columns for region clicks and bitmap region outputs

DO $$
BEGIN
    -- 1. Ensure 'region' is allowed in automation_variables check constraint
    IF EXISTS (
        SELECT 1 FROM information_schema.check_constraints
        WHERE constraint_name = 'automation_variables_var_type_check'
    ) THEN
        ALTER TABLE automation_variables DROP CONSTRAINT automation_variables_var_type_check;
        ALTER TABLE automation_variables ADD CONSTRAINT automation_variables_var_type_check
            CHECK (var_type IN ('int', 'bool', 'color', 'point', 'string', 'region'));
    END IF;

    -- 2. Ensure region_variable_id exists on step_mouse_clicks
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'step_mouse_clicks' AND column_name = 'region_variable_id'
    ) THEN
        ALTER TABLE step_mouse_clicks ADD COLUMN region_variable_id BIGINT REFERENCES automation_variables(id);
    END IF;

    -- 3. Ensure output_region_variable_id exists on step_find_bitmap
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'step_find_bitmap' AND column_name = 'output_region_variable_id'
    ) THEN
        ALTER TABLE step_find_bitmap ADD COLUMN output_region_variable_id BIGINT REFERENCES automation_variables(id);
    END IF;
END $$;
