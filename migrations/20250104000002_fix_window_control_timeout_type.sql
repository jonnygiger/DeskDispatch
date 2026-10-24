-- Align step_window_controls.timeout_ms with the rest of the schema and the
-- worker protocol, which represent timeouts as 64-bit milliseconds.
ALTER TABLE step_window_controls
    ALTER COLUMN timeout_ms TYPE BIGINT;
