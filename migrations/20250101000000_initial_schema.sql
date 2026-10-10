-- Consolidated Initial Schema Migration for DeskDispatch

-- 1. Identity & Access Control
CREATE TABLE users (
    id                   BIGSERIAL PRIMARY KEY,
    username             TEXT NOT NULL UNIQUE,
    password_hash        TEXT NOT NULL,
    display_name         TEXT NOT NULL,
    role                 TEXT NOT NULL CHECK (role IN ('admin', 'editor', 'viewer')),
    is_active            BOOLEAN NOT NULL DEFAULT true,
    must_change_password BOOLEAN NOT NULL DEFAULT false,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at        TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_users_lower_username ON users (lower(username));

CREATE TABLE sessions (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at     TIMESTAMPTZ NOT NULL,
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ip_address     INET,
    user_agent     TEXT
);

CREATE INDEX idx_sessions_user_expires ON sessions (user_id, expires_at);

CREATE TABLE audit_log (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT REFERENCES users(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id   BIGINT,
    details     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_audit_log_created_at ON audit_log (created_at DESC);

-- 2. Automations & Base Steps
CREATE TABLE automations (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'active', 'archived')),
    created_by  BIGINT NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE automation_steps (
    id            BIGSERIAL PRIMARY KEY,
    automation_id BIGINT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    position      DOUBLE PRECISION NOT NULL,
    step_type     TEXT NOT NULL CHECK (step_type IN ('mouse_click', 'key_press', 'find_pixel_rgb', 'find_bitmap', 'branch')),
    label         TEXT,
    post_delay_ms INTEGER NOT NULL DEFAULT 0 CHECK (post_delay_ms >= 0),
    deleted_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE automation_steps
    ADD CONSTRAINT automation_steps_automation_id_position_key
    UNIQUE (automation_id, position)
    DEFERRABLE INITIALLY DEFERRED;

CREATE UNIQUE INDEX idx_automation_steps_active_position
    ON automation_steps (automation_id, position)
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX idx_automation_steps_automation_id_id
    ON automation_steps (automation_id, id);

CREATE INDEX idx_steps_automation_position ON automation_steps (automation_id, position);

-- 3. Dynamic Variables, Parameters, Reference Bitmaps & Screenshots
CREATE TABLE automation_variables (
    id            BIGSERIAL PRIMARY KEY,
    automation_id BIGINT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    var_type      TEXT NOT NULL CHECK (var_type IN ('int', 'bool', 'color', 'point', 'string')),
    description   TEXT NOT NULL DEFAULT '',
    UNIQUE (automation_id, name)
);

CREATE TABLE automation_parameters (
    id            BIGSERIAL PRIMARY KEY,
    automation_id BIGINT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    param_type    TEXT NOT NULL CHECK (param_type IN ('int', 'bool', 'color', 'string')),
    default_value TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    UNIQUE (automation_id, name)
);

CREATE TABLE bitmaps (
    id                 BIGSERIAL PRIMARY KEY,
    automation_id      BIGINT REFERENCES automations(id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    object_storage_key TEXT NOT NULL,
    width              INTEGER NOT NULL CHECK (width > 0),
    height             INTEGER NOT NULL CHECK (height > 0),
    created_by         BIGINT NOT NULL REFERENCES users(id),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE step_screenshots (
    step_id            BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    object_storage_key TEXT NOT NULL,
    width              INTEGER NOT NULL CHECK (width > 0),
    height             INTEGER NOT NULL CHECK (height > 0),
    captured_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 4. Step Detail Tables
CREATE TABLE step_mouse_clicks (
    step_id       BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    x             INTEGER CHECK (x IS NULL OR x >= 0),
    y             INTEGER CHECK (y IS NULL OR y >= 0),
    x_variable_id BIGINT REFERENCES automation_variables(id),
    y_variable_id BIGINT REFERENCES automation_variables(id),
    button        TEXT NOT NULL DEFAULT 'left' CHECK (button IN ('left', 'right', 'middle')),
    click_type    TEXT NOT NULL DEFAULT 'single' CHECK (click_type IN ('single', 'double')),
    CHECK ((x IS NOT NULL) <> (x_variable_id IS NOT NULL)),
    CHECK ((y IS NOT NULL) <> (y_variable_id IS NOT NULL))
);

CREATE TABLE step_key_presses (
    step_id   BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    key_combo TEXT NOT NULL
);

CREATE TABLE step_find_pixel_rgb (
    step_id            BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    x                  INTEGER NOT NULL CHECK (x >= 0),
    y                  INTEGER NOT NULL CHECK (y >= 0),
    output_variable_id BIGINT REFERENCES automation_variables(id)
);

CREATE TABLE step_find_bitmap (
    step_id                  BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    reference_bitmap_id      BIGINT NOT NULL REFERENCES bitmaps(id),
    search_x                 INTEGER CHECK (search_x IS NULL OR search_x >= 0),
    search_y                 INTEGER CHECK (search_y IS NULL OR search_y >= 0),
    search_width             INTEGER CHECK (search_width IS NULL OR search_width > 0),
    search_height            INTEGER CHECK (search_height IS NULL OR search_height > 0),
    match_threshold          REAL NOT NULL DEFAULT 0.95 CHECK (match_threshold >= 0.0 AND match_threshold <= 1.0),
    output_found_variable_id BIGINT REFERENCES automation_variables(id),
    output_x_variable_id     BIGINT REFERENCES automation_variables(id),
    output_y_variable_id     BIGINT REFERENCES automation_variables(id)
);

CREATE TABLE step_branches (
    step_id             BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    automation_id       BIGINT NOT NULL,
    condition_type      TEXT NOT NULL CHECK (condition_type IN ('pixel_rgb', 'bitmap')),
    x                   INTEGER CHECK (x IS NULL OR x >= 0),
    y                   INTEGER CHECK (y IS NULL OR y >= 0),
    expected_r          SMALLINT CHECK (expected_r IS NULL OR (expected_r >= 0 AND expected_r <= 255)),
    expected_g          SMALLINT CHECK (expected_g IS NULL OR (expected_g >= 0 AND expected_g <= 255)),
    expected_b          SMALLINT CHECK (expected_b IS NULL OR (expected_b >= 0 AND expected_b <= 255)),
    tolerance           SMALLINT CHECK (tolerance IS NULL OR (tolerance >= 0 AND tolerance <= 255)),
    reference_bitmap_id BIGINT REFERENCES bitmaps(id),
    search_x            INTEGER CHECK (search_x IS NULL OR search_x >= 0),
    search_y            INTEGER CHECK (search_y IS NULL OR search_y >= 0),
    search_width        INTEGER CHECK (search_width IS NULL OR search_width > 0),
    search_height       INTEGER CHECK (search_height IS NULL OR search_height > 0),
    match_threshold     REAL CHECK (match_threshold IS NULL OR (match_threshold >= 0.0 AND match_threshold <= 1.0)),
    on_match_step_id    BIGINT NOT NULL,
    on_no_match_step_id BIGINT NOT NULL,
    CHECK (
      (condition_type = 'pixel_rgb' AND x IS NOT NULL AND reference_bitmap_id IS NULL) OR
      (condition_type = 'bitmap' AND reference_bitmap_id IS NOT NULL AND x IS NULL)
    ),
    CONSTRAINT step_branches_step_automation_fkey
        FOREIGN KEY (step_id, automation_id)
        REFERENCES automation_steps(id, automation_id)
        ON DELETE CASCADE,
    CONSTRAINT step_branches_on_match_fk
        FOREIGN KEY (automation_id, on_match_step_id)
        REFERENCES automation_steps(automation_id, id),
    CONSTRAINT step_branches_on_no_match_fk
        FOREIGN KEY (automation_id, on_no_match_step_id)
        REFERENCES automation_steps(automation_id, id)
);

CREATE INDEX idx_step_branches_on_match_step_id ON step_branches (on_match_step_id);
CREATE INDEX idx_step_branches_on_no_match_step_id ON step_branches (on_no_match_step_id);

-- 5. Task Worker PCs & Worker Groups
CREATE TABLE task_worker_pcs (
    id                            BIGSERIAL PRIMARY KEY,
    hostname                      TEXT NOT NULL,
    display_name                  TEXT NOT NULL,
    api_key_hash                  BYTEA,
    status                        TEXT NOT NULL DEFAULT 'offline'
                                    CHECK (status IN ('offline', 'online', 'busy', 'error')),
    last_heartbeat_at             TIMESTAMPTZ,
    screen_width                  INTEGER CHECK (screen_width IS NULL OR screen_width > 0),
    screen_height                 INTEGER CHECK (screen_height IS NULL OR screen_height > 0),
    os_info                       TEXT,
    agent_version                 TEXT,
    registration_token_hash       BYTEA UNIQUE,
    registration_token_expires_at TIMESTAMPTZ,
    previous_api_key_hash         BYTEA,
    previous_api_key_expires_at   TIMESTAMPTZ,
    created_at                    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_task_worker_pcs_api_key_hash
    ON task_worker_pcs (api_key_hash)
    WHERE api_key_hash IS NOT NULL;

CREATE INDEX idx_task_worker_pcs_prev_api_key_hash
    ON task_worker_pcs (previous_api_key_hash)
    WHERE previous_api_key_hash IS NOT NULL;

CREATE TABLE worker_groups (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE worker_group_members (
    worker_id BIGINT NOT NULL REFERENCES task_worker_pcs(id) ON DELETE CASCADE,
    group_id  BIGINT NOT NULL REFERENCES worker_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (worker_id, group_id)
);

-- 6. Scheduling
CREATE TABLE schedules (
    id              BIGSERIAL PRIMARY KEY,
    automation_id   BIGINT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    cron_expression TEXT NOT NULL,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    worker_group_id BIGINT REFERENCES worker_groups(id),
    is_enabled      BOOLEAN NOT NULL DEFAULT true,
    overlap_policy  TEXT NOT NULL DEFAULT 'allow' CHECK (overlap_policy IN ('allow', 'skip', 'queue')),
    max_queue_age_secs INTEGER,
    next_run_at     TIMESTAMPTZ,
    last_run_at     TIMESTAMPTZ,
    created_by      BIGINT NOT NULL REFERENCES users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_schedules_due ON schedules (next_run_at) WHERE is_enabled;

-- 7. Task Execution & Runs
CREATE TABLE task_runs (
    id                         BIGSERIAL PRIMARY KEY,
    automation_id              BIGINT NOT NULL REFERENCES automations(id),
    schedule_id                BIGINT REFERENCES schedules(id),
    worker_id                  BIGINT REFERENCES task_worker_pcs(id),
    status                     TEXT NOT NULL DEFAULT 'queued'
                                 CHECK (status IN ('queued', 'running', 'cancelling', 'succeeded', 'failed', 'cancelled', 'lost')),
    triggered_by_user_id       BIGINT REFERENCES users(id),
    current_step_id            BIGINT REFERENCES automation_steps(id) ON DELETE SET NULL,
    queued_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at                 TIMESTAMPTZ,
    completed_at               TIMESTAMPTZ,
    error_message              TEXT,
    cancel_requested_at        TIMESTAMPTZ,
    target_worker_group_id     BIGINT REFERENCES worker_groups(id) ON DELETE SET NULL,
    dispatched_automation_json JSONB,
    parameter_overrides        JSONB
);

CREATE INDEX idx_task_runs_queued ON task_runs (queued_at, id) WHERE status = 'queued';
CREATE UNIQUE INDEX idx_task_runs_worker_running ON task_runs (worker_id) WHERE status IN ('running', 'cancelling');
CREATE INDEX idx_task_runs_target_worker_group ON task_runs (target_worker_group_id) WHERE status = 'queued';
CREATE INDEX idx_task_runs_automation_worker ON task_runs (automation_id, worker_id);
CREATE INDEX idx_task_runs_worker_id ON task_runs (worker_id);
CREATE INDEX idx_task_runs_queued_at_id ON task_runs (queued_at DESC, id DESC);
CREATE INDEX idx_task_runs_schedule_id ON task_runs (schedule_id);

CREATE TABLE task_run_steps (
    id                    BIGSERIAL PRIMARY KEY,
    task_run_id           BIGINT NOT NULL REFERENCES task_runs(id) ON DELETE CASCADE,
    step_id               BIGINT NOT NULL REFERENCES automation_steps(id),
    seq                   INTEGER,
    started_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at          TIMESTAMPTZ,
    result                TEXT CHECK (result IN ('success', 'failed', 'branch_matched', 'branch_not_matched')),
    captured_r            SMALLINT CHECK (captured_r IS NULL OR (captured_r >= 0 AND captured_r <= 255)),
    captured_g            SMALLINT CHECK (captured_g IS NULL OR (captured_g >= 0 AND captured_g <= 255)),
    captured_b            SMALLINT CHECK (captured_b IS NULL OR (captured_b >= 0 AND captured_b <= 255)),
    captured_found        BOOLEAN,
    captured_x            INTEGER CHECK (captured_x IS NULL OR captured_x >= 0),
    captured_y            INTEGER CHECK (captured_y IS NULL OR captured_y >= 0),
    screenshot_object_key TEXT
);

CREATE UNIQUE INDEX idx_task_run_steps_run_seq ON task_run_steps (task_run_id, seq);
CREATE INDEX idx_task_run_steps_task_run_id ON task_run_steps (task_run_id, id);

CREATE TABLE task_run_variable_values (
    task_run_id    BIGINT NOT NULL REFERENCES task_runs(id) ON DELETE CASCADE,
    variable_id    BIGINT NOT NULL REFERENCES automation_variables(id),
    value          TEXT NOT NULL,
    set_at_step_id BIGINT REFERENCES automation_steps(id) ON DELETE SET NULL,
    set_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (task_run_id, variable_id)
);

-- 8. Recording Sessions & Events
CREATE TABLE recording_sessions (
    id                      BIGSERIAL PRIMARY KEY,
    worker_id               BIGINT NOT NULL REFERENCES task_worker_pcs(id),
    started_by_user_id      BIGINT NOT NULL REFERENCES users(id),
    status                  TEXT NOT NULL DEFAULT 'recording'
                              CHECK (status IN ('recording', 'completed', 'imported', 'discarded')),
    resulting_automation_id BIGINT REFERENCES automations(id),
    started_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at                TIMESTAMPTZ
);

CREATE TABLE recording_events (
    id                   BIGSERIAL PRIMARY KEY,
    recording_session_id BIGINT NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
    sequence_number      INTEGER NOT NULL,
    event_type           TEXT NOT NULL CHECK (event_type IN ('mouse_click', 'key_press', 'screenshot')),
    x                    INTEGER CHECK (x IS NULL OR x >= 0),
    y                    INTEGER CHECK (y IS NULL OR y >= 0),
    button               TEXT,
    key_combo            TEXT,
    screenshot_object_key TEXT,
    captured_at          TIMESTAMPTZ NOT NULL,
    UNIQUE (recording_session_id, sequence_number)
);

-- 9. LISTEN / NOTIFY Triggers & Functions
CREATE OR REPLACE FUNCTION notify_task_queue_changed()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('task_queue_changed', '');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_notify_task_runs_queued
AFTER INSERT OR UPDATE OF status ON task_runs
FOR EACH ROW
WHEN (NEW.status = 'queued')
EXECUTE FUNCTION notify_task_queue_changed();

CREATE TRIGGER trg_notify_recording_sessions ON recording_sessions
AFTER INSERT OR UPDATE OF status ON recording_sessions
FOR EACH ROW
WHEN (NEW.status = 'recording')
EXECUTE FUNCTION notify_task_queue_changed();
