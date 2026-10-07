-- Make automation_steps (automation_id, position) unique constraint deferrable
ALTER TABLE automation_steps
    DROP CONSTRAINT automation_steps_automation_id_position_key;

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_automation_id_position_key
    UNIQUE (automation_id, position)
    DEFERRABLE INITIALLY DEFERRED;
