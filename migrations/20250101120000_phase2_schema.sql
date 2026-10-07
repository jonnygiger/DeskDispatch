-- Phase 2 Schema & Migrations

-- 1. Identity & Access Control
CREATE TABLE users (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'editor', 'viewer')),
    is_active     BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ
);

CREATE TABLE sessions (
    id         UUID PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    ip_address INET,
    user_agent TEXT
);

CREATE TABLE audit_log (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT REFERENCES users(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id   BIGINT,
    details     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

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
    post_delay_ms INTEGER NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (automation_id, position)
);

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
    width              INTEGER NOT NULL,
    height             INTEGER NOT NULL,
    created_by         BIGINT NOT NULL REFERENCES users(id),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE step_screenshots (
    step_id            BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    object_storage_key TEXT NOT NULL,
    width              INTEGER NOT NULL,
    height             INTEGER NOT NULL,
    captured_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 4. Step Detail Tables
CREATE TABLE step_mouse_clicks (
    step_id       BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    x             INTEGER,
    y             INTEGER,
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
    x                  INTEGER NOT NULL,
    y                  INTEGER NOT NULL,
    output_variable_id BIGINT REFERENCES automation_variables(id)
);

CREATE TABLE step_find_bitmap (
    step_id                  BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    reference_bitmap_id      BIGINT NOT NULL REFERENCES bitmaps(id),
    search_x                 INTEGER,
    search_y                 INTEGER,
    search_width             INTEGER,
    search_height            INTEGER,
    match_threshold          REAL NOT NULL DEFAULT 0.95,
    output_found_variable_id BIGINT REFERENCES automation_variables(id),
    output_x_variable_id     BIGINT REFERENCES automation_variables(id),
    output_y_variable_id     BIGINT REFERENCES automation_variables(id)
);

CREATE TABLE step_branches (
    step_id             BIGINT PRIMARY KEY REFERENCES automation_steps(id) ON DELETE CASCADE,
    condition_type      TEXT NOT NULL CHECK (condition_type IN ('pixel_rgb', 'bitmap')),
    x                   INTEGER,
    y                   INTEGER,
    expected_r          SMALLINT,
    expected_g          SMALLINT,
    expected_b          SMALLINT,
    tolerance           SMALLINT,
    reference_bitmap_id BIGINT REFERENCES bitmaps(id),
    search_x            INTEGER,
    search_y            INTEGER,
    search_width        INTEGER,
    search_height       INTEGER,
    match_threshold     REAL,
    on_match_step_id    BIGINT REFERENCES automation_steps(id),
    on_no_match_step_id BIGINT REFERENCES automation_steps(id),
    CHECK (
      (condition_type = 'pixel_rgb' AND x IS NOT NULL AND reference_bitmap_id IS NULL) OR
      (condition_type = 'bitmap' AND reference_bitmap_id IS NOT NULL AND x IS NULL)
    )
);

-- 5. Task Worker PCs & Worker Groups
CREATE TABLE task_worker_pcs (
    id                 BIGSERIAL PRIMARY KEY,
    hostname           TEXT NOT NULL,
    display_name       TEXT NOT NULL,
    api_key_hash       BYTEA NOT NULL,
    registration_token TEXT UNIQUE,
    status             TEXT NOT NULL DEFAULT 'offline'
                         CHECK (status IN ('offline', 'online', 'busy', 'error')),
    last_heartbeat_at  TIMESTAMPTZ,
    screen_width       INTEGER,
    screen_height      INTEGER,
    os_info            TEXT,
    agent_version      TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

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
    next_run_at     TIMESTAMPTZ,
    last_run_at     TIMESTAMPTZ,
    created_by      BIGINT NOT NULL REFERENCES users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 7. Task Execution & Runs
CREATE TABLE task_runs (
    id                   BIGSERIAL PRIMARY KEY,
    automation_id        BIGINT NOT NULL REFERENCES automations(id),
    schedule_id          BIGINT REFERENCES schedules(id),
    worker_id            BIGINT REFERENCES task_worker_pcs(id),
    status               TEXT NOT NULL DEFAULT 'queued'
                           CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'lost')),
    triggered_by_user_id BIGINT REFERENCES users(id),
    current_step_id      BIGINT REFERENCES automation_steps(id),
    queued_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at           TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    error_message        TEXT
);

CREATE TABLE task_run_steps (
    id                    BIGSERIAL PRIMARY KEY,
    task_run_id           BIGINT NOT NULL REFERENCES task_runs(id) ON DELETE CASCADE,
    step_id               BIGINT NOT NULL REFERENCES automation_steps(id),
    started_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at          TIMESTAMPTZ,
    result                TEXT CHECK (result IN ('success', 'failed', 'branch_matched', 'branch_not_matched')),
    captured_r            SMALLINT,
    captured_g            SMALLINT,
    captured_b            SMALLINT,
    captured_found        BOOLEAN,
    captured_x            INTEGER,
    captured_y            INTEGER,
    screenshot_object_key TEXT
);

CREATE TABLE task_run_variable_values (
    task_run_id    BIGINT NOT NULL REFERENCES task_runs(id) ON DELETE CASCADE,
    variable_id    BIGINT NOT NULL REFERENCES automation_variables(id),
    value          TEXT NOT NULL,
    set_at_step_id BIGINT REFERENCES automation_steps(id),
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
    x                    INTEGER,
    y                    INTEGER,
    button               TEXT,
    key_combo            TEXT,
    screenshot_object_key TEXT,
    captured_at          TIMESTAMPTZ NOT NULL,
    UNIQUE (recording_session_id, sequence_number)
);

-- 9. Performance Indexes
CREATE INDEX idx_steps_automation_position ON automation_steps (automation_id, position);
CREATE INDEX idx_schedules_due ON schedules (next_run_at) WHERE is_enabled;
CREATE INDEX idx_task_runs_queued ON task_runs (status, queued_at) WHERE status = 'queued';
