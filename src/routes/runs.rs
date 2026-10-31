use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{AuthUser, CsrfForm, RequireEditor, log_audit};
use crate::de::deserialize_option_number;
use crate::magnifier::ImageMagnifier;
use axum::Router;
use axum::routing::{get, post};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/runs", get(get_runs_handler))
        .route("/runs/{id}", get(get_run_detail_handler))
        .route(
            "/runs/{id}/screenshots.zip",
            get(get_run_screenshots_zip_handler),
        )
        .route("/runs/{id}/status-frame", get(get_run_status_frame_handler))
        .route("/runs/{id}/cancel", post(post_cancel_run_handler))
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RunsFilterQuery {
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub automation_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub worker_id: Option<i64>,
    pub status: Option<String>,
    pub before_queued_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub before_id: Option<i64>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct StatusFrameQuery {
    pub paused: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct CancelRunForm {
    pub csrf_token: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskRunListItem {
    pub id: i64,
    pub automation_id: i64,
    pub automation_name: String,
    pub schedule_id: Option<i64>,
    pub schedule_name: Option<String>,
    pub worker_id: Option<i64>,
    pub worker_name: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub queued_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error_message: Option<String>,
    pub screenshot_step_id: Option<i64>,
    pub output_type: Option<String>,
    pub output_value: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct TaskRunDetailRow {
    id: i64,
    automation_id: i64,
    automation_name: String,
    schedule_id: Option<i64>,
    schedule_name: Option<String>,
    worker_id: Option<i64>,
    worker_name: Option<String>,
    worker_screen_width: Option<i32>,
    worker_screen_height: Option<i32>,
    status: String,
    triggered_by: String,
    queued_at: chrono::DateTime<chrono::Utc>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    error_message: Option<String>,
    output_type: Option<String>,
    output_value: Option<String>,
    parameter_overrides: Option<serde_json::Value>,
    dispatched_automation_json:
        Option<sqlx::types::Json<deskdispatch_protocol::FullAutomationPayload>>,
}

impl TaskRunDetailRow {
    fn into_item(self) -> TaskRunListItem {
        TaskRunListItem {
            id: self.id,
            automation_id: self.automation_id,
            automation_name: self.automation_name,
            schedule_id: self.schedule_id,
            schedule_name: self.schedule_name,
            worker_id: self.worker_id,
            worker_name: self.worker_name,
            status: self.status,
            triggered_by: self.triggered_by,
            queued_at: self.queued_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
            error_message: self.error_message,
            screenshot_step_id: None,
            output_type: self.output_type,
            output_value: self.output_value,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ExecutedStepRow {
    id: i64,
    step_id: i64,
    step_type: String,
    label: Option<String>,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    result: Option<String>,
    captured_r: Option<i16>,
    captured_g: Option<i16>,
    captured_b: Option<i16>,
    captured_found: Option<bool>,
    captured_x: Option<i32>,
    captured_y: Option<i32>,
    screenshot_object_key: Option<String>,
    step_screenshot_key: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct RunVariableValueRow {
    variable_id: i64,
    variable_name: String,
    var_type: String,
    value: Option<String>,
    set_at_step_id: Option<i64>,
    set_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ParamDefaultRow {
    name: String,
    default_value: String,
}

#[derive(Debug, sqlx::FromRow)]
struct ZipStepRow {
    seq: Option<i32>,
    step_id: i64,
    screenshot_object_key: Option<String>,
    step_screenshot_key: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CancelRunRow {
    automation_id: i64,
    worker_id: Option<i64>,
}

impl TaskRunListItem {
    pub fn formatted_queued_at(&self) -> String {
        self.queued_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    pub fn formatted_started_at(&self) -> String {
        self.started_at
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string())
    }

    pub fn formatted_completed_at(&self) -> String {
        self.completed_at
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string())
    }

    pub fn duration_display(&self) -> String {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => {
                let duration = end.signed_duration_since(start);
                let secs = duration.num_seconds();
                if secs < 60 {
                    format!("{}s", secs)
                } else if secs < 3600 {
                    format!("{}m {}s", secs / 60, secs % 60)
                } else {
                    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                }
            }
            (Some(start), None) => {
                let duration = chrono::Utc::now().signed_duration_since(start);
                format!("{}s (running)", duration.num_seconds().max(0))
            }
            _ => "-".to_string(),
        }
    }

    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "queued" => "badge-warning",
            "running" => "badge-info",
            "cancelling" => "badge-warning",
            "succeeded" => "badge-success",
            "failed" => "badge-danger",
            "cancelled" => "badge-secondary",
            "lost" => "badge-danger",
            _ => "badge-secondary",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AutomationOption {
    pub id: i64,
    pub name: String,
}

impl AutomationOption {
    pub fn is_selected(&self, filter_id: &Option<i64>) -> bool {
        *filter_id == Some(self.id)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WorkerOption {
    pub id: i64,
    pub display_name: String,
}

impl WorkerOption {
    pub fn is_selected(&self, filter_id: &Option<i64>) -> bool {
        *filter_id == Some(self.id)
    }
}

#[derive(Template)]
#[template(path = "runs/list.html")]
pub struct RunsListTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub runs: Vec<TaskRunListItem>,
    pub automations: Vec<AutomationOption>,
    pub workers: Vec<WorkerOption>,
    pub filter_automation_id: Option<i64>,
    pub filter_worker_id: Option<i64>,
    pub filter_status: Option<String>,
    pub next_queued_at: Option<String>,
    pub next_id: Option<i64>,
    pub has_next_page: bool,
}

impl RunsListTemplate {
    pub fn is_status_selected(&self, expected: &str) -> bool {
        match self.filter_status.as_deref() {
            Some(actual) => actual == expected,
            None => expected == "all",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutedStepItem {
    pub id: i64,
    pub step_id: i64,
    pub step_number: usize,
    pub step_type: String,
    pub label: Option<String>,
    pub result: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub captured_r: Option<i16>,
    pub captured_g: Option<i16>,
    pub captured_b: Option<i16>,
    pub captured_found: Option<bool>,
    pub captured_x: Option<i32>,
    pub captured_y: Option<i32>,
    pub screenshot_object_key: Option<String>,
    pub magnifier: Option<ImageMagnifier>,
}

impl ExecutedStepItem {
    pub fn formatted_started_at(&self) -> String {
        self.started_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    pub fn formatted_completed_at(&self) -> String {
        self.completed_at
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string())
    }

    pub fn result_badge_class(&self) -> &'static str {
        match self.result.as_deref() {
            Some("success") | Some("branch_matched") => "badge-success",
            Some("branch_not_matched") => "badge-info",
            Some("failed") => "badge-danger",
            _ => "badge-secondary",
        }
    }

    pub fn captured_rgb_display(&self) -> Option<String> {
        match (self.captured_r, self.captured_g, self.captured_b) {
            (Some(r), Some(g), Some(b)) => Some(format!("RGB({}, {}, {})", r, g, b)),
            _ => None,
        }
    }

    pub fn captured_xy_display(&self) -> Option<String> {
        match (self.captured_x, self.captured_y) {
            (Some(x), Some(y)) => Some(format!("({}, {})", x, y)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunVariableValueItem {
    pub variable_id: i64,
    pub variable_name: String,
    pub var_type: String,
    pub value: String,
    pub set_at_step_id: Option<i64>,
    pub set_at: chrono::DateTime<chrono::Utc>,
}

impl RunVariableValueItem {
    pub fn formatted_set_at(&self) -> String {
        self.set_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

#[derive(Debug, Clone)]
pub struct RunParameterItem {
    pub name: String,
    pub value: String,
    pub is_override: bool,
}

#[derive(Template)]
#[template(path = "runs/detail.html")]
pub struct RunDetailTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub run: TaskRunListItem,
    pub steps: Vec<ExecutedStepItem>,
    pub variable_values: Vec<RunVariableValueItem>,
    pub parameters: Vec<RunParameterItem>,
    pub auto_refresh: bool,
}

#[derive(Template)]
#[template(path = "runs/status_frame.html")]
pub struct RunStatusFrameTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub run: TaskRunListItem,
    pub auto_refresh: bool,
    pub paused: bool,
}

#[tracing::instrument(skip(state, user))]
pub async fn get_runs_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Query(filter): Query<RunsFilterQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let filter_status_clean = filter
        .status
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "all");

    // Bolt Optimization: Run all 3 independent queries (automations filter list, workers filter list,
    // and filtered task runs list) concurrently using `tokio::join!`. This reduces page response latency
    // from the cumulative sum of 3 sequential database round-trips to the duration of the single longest query.
    let (auto_rows_res, worker_rows_res, run_rows_res) = tokio::join!(
        sqlx::query_as::<_, AutomationOption>("SELECT id, name FROM automations ORDER BY name ASC")
            .fetch_all(&state.db),
        sqlx::query_as::<_, WorkerOption>(
            "SELECT id, display_name FROM task_worker_pcs ORDER BY display_name ASC"
        )
        .fetch_all(&state.db),
        sqlx::query_as::<_, TaskRunListItem>(
            r#"
            SELECT
                tr.id,
                tr.automation_id,
                a.name AS automation_name,
                tr.schedule_id,
                s.name AS schedule_name,
                tr.worker_id,
                w.display_name AS worker_name,
                tr.status,
                COALESCE(u.display_name, u.username, s.name, 'Manual') AS triggered_by,
                tr.queued_at,
                tr.started_at,
                tr.completed_at,
                tr.error_message,
                ss.step_id AS screenshot_step_id,
                tr.output_type,
                tr.output_value
            FROM task_runs tr
            JOIN automations a ON tr.automation_id = a.id
            LEFT JOIN schedules s ON tr.schedule_id = s.id
            LEFT JOIN task_worker_pcs w ON tr.worker_id = w.id
            LEFT JOIN users u ON tr.triggered_by_user_id = u.id
            LEFT JOIN LATERAL (
                SELECT trs.step_id
                FROM task_run_steps trs
                WHERE trs.task_run_id = tr.id AND trs.screenshot_object_key IS NOT NULL
                ORDER BY trs.id DESC
                LIMIT 1
            ) ss ON true
            WHERE ($1::BIGINT IS NULL OR tr.automation_id = $1)
              AND ($2::BIGINT IS NULL OR tr.worker_id = $2)
              AND ($3::TEXT IS NULL OR tr.status = $3)
              AND (
                ($4::TIMESTAMPTZ IS NULL AND $5::BIGINT IS NULL)
                OR (tr.queued_at, tr.id) < ($4, $5)
              )
            ORDER BY tr.queued_at DESC, tr.id DESC
            LIMIT 51
            "#,
        )
        .bind(filter.automation_id)
        .bind(filter.worker_id)
        .bind(filter_status_clean.as_deref())
        .bind(filter.before_queued_at)
        .bind(filter.before_id)
        .fetch_all(&state.db)
    );

    let automations = auto_rows_res.unwrap_or_default();
    let workers = worker_rows_res.unwrap_or_default();

    let mut run_rows = run_rows_res.unwrap_or_default();
    let limit = 50;
    let has_next_page = run_rows.len() > limit;
    if has_next_page {
        run_rows.truncate(limit);
    }
    let runs = run_rows;

    let (next_queued_at, next_id) = if has_next_page {
        if let Some(last_item) = runs.last() {
            (Some(last_item.queued_at.to_rfc3339()), Some(last_item.id))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    HtmlTemplate(RunsListTemplate {
        user,
        csrf_token,
        runs,
        automations,
        workers,
        filter_automation_id: filter.automation_id,
        filter_worker_id: filter.worker_id,
        filter_status: filter_status_clean,
        next_queued_at,
        next_id,
        has_next_page,
    })
}

#[tracing::instrument(skip(state, user))]
pub async fn get_run_status_frame_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<StatusFrameQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let run_row = match sqlx::query_as::<_, TaskRunDetailRow>(
        r#"
        SELECT
            tr.id,
            tr.automation_id,
            a.name AS automation_name,
            tr.schedule_id,
            s.name AS schedule_name,
            tr.worker_id,
            w.display_name AS worker_name,
            w.screen_width AS worker_screen_width,
            w.screen_height AS worker_screen_height,
            tr.status,
            COALESCE(u.display_name, u.username, s.name, 'Manual') AS triggered_by,
            tr.queued_at,
            tr.started_at,
            tr.completed_at,
            tr.error_message,
            tr.output_type,
            tr.output_value,
            tr.parameter_overrides,
            tr.dispatched_automation_json
        FROM task_runs tr
        JOIN automations a ON tr.automation_id = a.id
        LEFT JOIN schedules s ON tr.schedule_id = s.id
        LEFT JOIN task_worker_pcs w ON tr.worker_id = w.id
        LEFT JOIN users u ON tr.triggered_by_user_id = u.id
        WHERE tr.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        _ => return (StatusCode::NOT_FOUND, "Task run not found").into_response(),
    };

    let run_item = run_row.into_item();

    let paused = query.paused.unwrap_or(false);
    let auto_refresh = (run_item.status == "queued"
        || run_item.status == "running"
        || run_item.status == "cancelling")
        && !paused;

    HtmlTemplate(RunStatusFrameTemplate {
        user,
        csrf_token,
        run: run_item,
        auto_refresh,
        paused,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn get_run_detail_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    // Bolt Optimization: Run all 3 independent task run detail queries (main task run record,
    // executed step logs with screenshot keys, and variable values) concurrently using `tokio::join!`.
    // This reduces page load database wait latency from 3 sequential round-trips to the duration
    // of the single longest query (~66% latency reduction).
    let (run_row_res, step_rows_res, var_rows_res) = tokio::join!(
        sqlx::query_as::<_, TaskRunDetailRow>(
            r#"
            SELECT
                tr.id,
                tr.automation_id,
                a.name AS automation_name,
                tr.schedule_id,
                s.name AS schedule_name,
                tr.worker_id,
                w.display_name AS worker_name,
                w.screen_width AS worker_screen_width,
                w.screen_height AS worker_screen_height,
                tr.status,
                COALESCE(u.display_name, u.username, s.name, 'Manual') AS triggered_by,
                tr.queued_at,
                tr.started_at,
                tr.completed_at,
                tr.error_message,
                tr.output_type,
                tr.output_value,
                tr.parameter_overrides,
                tr.dispatched_automation_json
            FROM task_runs tr
            JOIN automations a ON tr.automation_id = a.id
            LEFT JOIN schedules s ON tr.schedule_id = s.id
            LEFT JOIN task_worker_pcs w ON tr.worker_id = w.id
            LEFT JOIN users u ON tr.triggered_by_user_id = u.id
            WHERE tr.id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&state.db),
        sqlx::query_as::<_, ExecutedStepRow>(
            r#"
            SELECT
                trs.id,
                trs.step_id,
                s.step_type,
                s.label,
                trs.started_at,
                trs.completed_at,
                trs.result,
                trs.captured_r,
                trs.captured_g,
                trs.captured_b,
                trs.captured_found,
                trs.captured_x,
                trs.captured_y,
                trs.screenshot_object_key,
                ss.object_storage_key AS step_screenshot_key
            FROM task_run_steps trs
            JOIN automation_steps s ON trs.step_id = s.id
            LEFT JOIN step_screenshots ss ON s.id = ss.step_id
            WHERE trs.task_run_id = $1
            ORDER BY trs.id ASC
            "#,
        )
        .bind(id)
        .fetch_all(&state.db),
        sqlx::query_as::<_, RunVariableValueRow>(
            r#"
            SELECT
                trvv.variable_id,
                av.name AS variable_name,
                av.var_type,
                trvv.value,
                trvv.set_at_step_id,
                trvv.set_at
            FROM task_run_variable_values trvv
            JOIN automation_variables av ON trvv.variable_id = av.id
            WHERE trvv.task_run_id = $1
            ORDER BY av.name ASC
            "#,
        )
        .bind(id)
        .fetch_all(&state.db),
    );

    let run_row = match run_row_res {
        Ok(Some(row)) => row,
        _ => return Redirect::to("/runs").into_response(),
    };

    let worker_screen_width: u32 = run_row.worker_screen_width.unwrap_or(1920).max(1) as u32;
    let worker_screen_height: u32 = run_row.worker_screen_height.unwrap_or(1080).max(1) as u32;

    let parameter_overrides_json = run_row.parameter_overrides.clone();
    let dispatched_json = run_row
        .dispatched_automation_json
        .as_ref()
        .map(|j| serde_json::to_value(&j.0).unwrap_or_default());

    let run_item = run_row.into_item();

    let auto_refresh = run_item.status == "queued"
        || run_item.status == "running"
        || run_item.status == "cancelling";

    let step_rows = step_rows_res.unwrap_or_default();
    let var_rows = var_rows_res.unwrap_or_default();

    let mut executed_steps = Vec::with_capacity(step_rows.len());
    for (idx, r) in step_rows.into_iter().enumerate() {
        let screenshot_key = r.screenshot_object_key.or(r.step_screenshot_key);

        let target_x = r.captured_x.map(|x| x.max(0) as u32);
        let target_y = r.captured_y.map(|y| y.max(0) as u32);

        let magnifier = if let Some(ref key) = screenshot_key {
            let presigned_url = match state
                .storage_service()
                .generate_presigned_get_url(key, Duration::from_secs(900))
            {
                Ok(url) => url,
                Err(_) => format!("/media/screenshots/{}", r.step_id),
            };

            Some(ImageMagnifier::new(
                presigned_url,
                worker_screen_width,
                worker_screen_height,
                target_x,
                target_y,
            ))
        } else {
            None
        };

        executed_steps.push(ExecutedStepItem {
            id: r.id,
            step_id: r.step_id,
            step_number: idx + 1,
            step_type: r.step_type,
            label: r.label,
            result: r.result,
            started_at: r.started_at,
            completed_at: r.completed_at,
            captured_r: r.captured_r,
            captured_g: r.captured_g,
            captured_b: r.captured_b,
            captured_found: r.captured_found,
            captured_x: r.captured_x,
            captured_y: r.captured_y,
            screenshot_object_key: screenshot_key,
            magnifier,
        });
    }

    let mut variable_values = Vec::with_capacity(var_rows.len());
    for r in var_rows {
        variable_values.push(RunVariableValueItem {
            variable_id: r.variable_id,
            variable_name: r.variable_name,
            var_type: r.var_type,
            value: r.value.unwrap_or_default(),
            set_at_step_id: r.set_at_step_id,
            set_at: r.set_at,
        });
    }

    let overrides_map = parameter_overrides_json
        .as_ref()
        .and_then(|v| v.as_object());

    let parameters = if let Some(params_obj) = dispatched_json
        .as_ref()
        .and_then(|j| j.get("parameters"))
        .and_then(|p| p.as_object())
    {
        let mut params = Vec::with_capacity(params_obj.len());
        for (k, v) in params_obj {
            let val_str = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => v.to_string(),
            };
            let is_override = overrides_map.is_some_and(|m| m.contains_key(k));
            params.push(RunParameterItem {
                name: k.clone(),
                value: val_str,
                is_override,
            });
        }
        params
    } else {
        let param_rows = sqlx::query_as::<_, ParamDefaultRow>(
            "SELECT name, default_value FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC",
        )
        .bind(run_item.automation_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let mut params = Vec::with_capacity(param_rows.len());
        for p in param_rows {
            let (val_str, is_override) = if let Some(override_val) = overrides_map
                .and_then(|m| m.get(&p.name))
                .and_then(|v| v.as_str())
            {
                (override_val.to_string(), true)
            } else {
                (p.default_value, false)
            };
            params.push(RunParameterItem {
                name: p.name,
                value: val_str,
                is_override,
            });
        }
        params
    };

    HtmlTemplate(RunDetailTemplate {
        user,
        csrf_token,
        run: run_item,
        steps: executed_steps,
        variable_values,
        parameters,
        auto_refresh,
    })
    .into_response()
}

#[tracing::instrument(skip(state, _user))]
pub async fn get_run_screenshots_zip_handler(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Response {
    let run_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_runs WHERE id = $1)")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap_or_default();

    if !run_exists {
        return (StatusCode::NOT_FOUND, "Task run not found").into_response();
    }

    let step_rows = match sqlx::query_as::<_, ZipStepRow>(
        r#"
        SELECT
            trs.seq,
            trs.step_id,
            trs.screenshot_object_key,
            ss.object_storage_key AS step_screenshot_key
        FROM task_run_steps trs
        LEFT JOIN step_screenshots ss ON trs.step_id = ss.step_id
        WHERE trs.task_run_id = $1
        ORDER BY trs.id ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to query run steps",
            )
                .into_response();
        }
    };

    let mut zip_builder = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    let storage = state.storage_service();
    let mut count = 0;

    for (idx, row) in step_rows.into_iter().enumerate() {
        let screenshot_key = row.screenshot_object_key.or(row.step_screenshot_key);

        if let Some(key) = screenshot_key
            && let Ok(bytes) = storage.get_object(&key).await
        {
            let filename = format!(
                "step_{:03}_seq_{}_step_{}.png",
                idx + 1,
                row.seq.unwrap_or((idx + 1) as i32),
                row.step_id
            );

            if zip_builder.start_file(&filename, zip_options).is_ok()
                && std::io::Write::write_all(&mut zip_builder, &bytes).is_ok()
            {
                count += 1;
            }
        }
    }

    if count == 0 {
        return (
            StatusCode::NOT_FOUND,
            "No screenshots found for this task run",
        )
            .into_response();
    }

    let buf = match zip_builder.finish() {
        Ok(cursor) => cursor.into_inner(),
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to finalize ZIP archive",
            )
                .into_response();
        }
    };
    let filename = format!("run-{}-screenshots.zip", id);
    (
        [
            ("Content-Type", "application/zip"),
            (
                "Content-Disposition",
                &format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        buf,
    )
        .into_response()
}

#[tracing::instrument(skip(state, user, _form))]
pub async fn post_cancel_run_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Path(id): Path<i64>,
    CsrfForm(_form): CsrfForm<CancelRunForm>,
) -> Response {
    let update_res = sqlx::query_as::<_, CancelRunRow>(
        r#"
        UPDATE task_runs
        SET status = CASE WHEN status = 'running' THEN 'cancelling' ELSE 'cancelled' END,
            cancel_requested_at = CASE WHEN status = 'running' THEN now() ELSE cancel_requested_at END,
            completed_at = CASE WHEN status = 'queued' THEN now() ELSE completed_at END
        WHERE id = $1 AND status IN ('queued', 'running')
        RETURNING automation_id, worker_id
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await;

    match update_res {
        Ok(Some(row)) => {
            let details = serde_json::json!({
                "automation_id": row.automation_id,
                "worker_id": row.worker_id,
            });

            let _ = log_audit(
                &state.db,
                Some(user.id),
                "cancel_task_run",
                "task_run",
                Some(id),
                Some(details),
            )
            .await;

            Redirect::to(&format!("/runs/{}", id)).into_response()
        }
        _ => Redirect::to(&format!("/runs/{}", id)).into_response(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_task_run_list_item_methods() {
        let now = chrono::Utc::now();
        let mut item = TaskRunListItem {
            id: 4821,
            automation_id: 12,
            automation_name: "Test Automation".to_string(),
            schedule_id: None,
            schedule_name: None,
            worker_id: Some(1),
            worker_name: Some("Worker-1".to_string()),
            status: "succeeded".to_string(),
            triggered_by: "Admin".to_string(),
            queued_at: now,
            started_at: Some(now),
            completed_at: Some(now + chrono::Duration::seconds(45)),
            error_message: None,
            screenshot_step_id: None,
            output_type: None,
            output_value: None,
        };

        assert_eq!(item.status_badge_class(), "badge-success");
        assert_eq!(item.duration_display(), "45s");
        assert!(!item.formatted_queued_at().is_empty());

        item.status = "cancelling".to_string();
        assert_eq!(item.status_badge_class(), "badge-warning");
    }

    #[test]
    fn test_executed_step_item_methods() {
        let now = chrono::Utc::now();
        let step_item = ExecutedStepItem {
            id: 1,
            step_id: 501,
            step_number: 1,
            step_type: "mouse_click".to_string(),
            label: Some("Click main button".to_string()),
            result: Some("success".to_string()),
            started_at: now,
            completed_at: Some(now + chrono::Duration::milliseconds(200)),
            captured_r: Some(40),
            captured_g: Some(180),
            captured_b: Some(60),
            captured_found: Some(true),
            captured_x: Some(824),
            captured_y: Some(391),
            screenshot_object_key: Some("runs/1/step_501.png".to_string()),
            magnifier: None,
        };

        assert_eq!(step_item.result_badge_class(), "badge-success");
        assert_eq!(
            step_item.captured_rgb_display(),
            Some("RGB(40, 180, 60)".to_string())
        );
        assert_eq!(
            step_item.captured_xy_display(),
            Some("(824, 391)".to_string())
        );
    }

    #[test]
    fn test_zip_writer() {
        let mut zip_builder = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip_builder.start_file("hello.txt", options).unwrap();
        std::io::Write::write_all(&mut zip_builder, b"Hello World!").unwrap();
        zip_builder.start_file("test.png", options).unwrap();
        std::io::Write::write_all(
            &mut zip_builder,
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();

        let zip_bytes = zip_builder.finish().unwrap().into_inner();
        assert!(!zip_bytes.is_empty());
        // Verify Local File Header Signature
        assert_eq!(&zip_bytes[0..4], &[0x50, 0x4B, 0x03, 0x04]);
    }

    #[test]
    fn test_task_run_formatting_and_badge_classes() {
        use crate::auth::UserRole;

        let now = chrono::Utc::now();

        // 1. TaskRunListItem duration display edge cases
        let mut item = TaskRunListItem {
            id: 1,
            automation_id: 1,
            automation_name: "Auto".to_string(),
            schedule_id: None,
            schedule_name: None,
            worker_id: None,
            worker_name: None,
            status: "succeeded".to_string(),
            triggered_by: "User".to_string(),
            queued_at: now,
            started_at: Some(now),
            completed_at: Some(now + chrono::Duration::seconds(125)), // 2m 5s
            error_message: None,
            screenshot_step_id: None,
            output_type: None,
            output_value: None,
        };
        assert_eq!(item.duration_display(), "2m 5s");

        item.completed_at = Some(now + chrono::Duration::seconds(3665)); // 1h 1m
        assert_eq!(item.duration_display(), "1h 1m");

        item.completed_at = None; // Running state
        assert!(item.duration_display().contains("(running)"));

        item.started_at = None; // Unstarted/Queued state
        assert_eq!(item.duration_display(), "-");
        assert_eq!(item.formatted_started_at(), "-");
        assert_eq!(item.formatted_completed_at(), "-");

        // 2. TaskRunListItem status badge classes
        let status_cases = [
            ("queued", "badge-warning"),
            ("running", "badge-info"),
            ("cancelling", "badge-warning"),
            ("succeeded", "badge-success"),
            ("failed", "badge-danger"),
            ("cancelled", "badge-secondary"),
            ("lost", "badge-danger"),
            ("unknown", "badge-secondary"),
        ];
        for (status, expected_class) in status_cases {
            item.status = status.to_string();
            assert_eq!(
                item.status_badge_class(),
                expected_class,
                "Status '{}' mismatch",
                status
            );
        }

        // 3. ExecutedStepItem result badge classes and partial captures
        let mut step = ExecutedStepItem {
            id: 1,
            step_id: 10,
            step_number: 1,
            step_type: "branch".to_string(),
            label: None,
            result: Some("branch_matched".to_string()),
            started_at: now,
            completed_at: None,
            captured_r: Some(255),
            captured_g: None,
            captured_b: Some(0),
            captured_found: None,
            captured_x: Some(100),
            captured_y: None,
            screenshot_object_key: None,
            magnifier: None,
        };

        assert_eq!(step.result_badge_class(), "badge-success");
        assert_eq!(step.formatted_completed_at(), "-");
        assert_eq!(step.captured_rgb_display(), None); // Incomplete RGB
        assert_eq!(step.captured_xy_display(), None); // Incomplete XY

        step.result = Some("branch_not_matched".to_string());
        assert_eq!(step.result_badge_class(), "badge-info");

        step.result = Some("failed".to_string());
        assert_eq!(step.result_badge_class(), "badge-danger");

        step.result = None;
        assert_eq!(step.result_badge_class(), "badge-secondary");

        // 4. RunsListTemplate filter selection logic
        let dummy_user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin".to_string(),
            role: UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "csrf".to_string(),
            must_change_password: false,
        };

        let list_tmpl = RunsListTemplate {
            user: dummy_user,
            csrf_token: "csrf".to_string(),
            runs: vec![],
            automations: vec![],
            workers: vec![],
            filter_automation_id: None,
            filter_worker_id: None,
            filter_status: Some("running".to_string()),
            next_queued_at: None,
            next_id: None,
            has_next_page: false,
        };

        assert!(list_tmpl.is_status_selected("running"));
        assert!(!list_tmpl.is_status_selected("succeeded"));

        let default_list_tmpl = RunsListTemplate {
            filter_status: None,
            ..list_tmpl
        };
        assert!(default_list_tmpl.is_status_selected("all"));
        assert!(!default_list_tmpl.is_status_selected("running"));

        // 5. AutomationOption and WorkerOption selection helpers
        let auto_opt = AutomationOption {
            id: 42,
            name: "Auto 42".to_string(),
        };
        assert!(auto_opt.is_selected(&Some(42)));
        assert!(!auto_opt.is_selected(&Some(10)));
        assert!(!auto_opt.is_selected(&None));

        let worker_opt = WorkerOption {
            id: 7,
            display_name: "Worker 7".to_string(),
        };
        assert!(worker_opt.is_selected(&Some(7)));
        assert!(!worker_opt.is_selected(&Some(1)));
        assert!(!worker_opt.is_selected(&None));
    }

    #[test]
    fn test_runs_filter_query_empty_string_deserialization() {
        let uri = format!("/runs?{}", "automation_id=&worker_id=&status=&before_id=")
            .parse::<axum::http::Uri>()
            .expect("Failed to parse URI");
        let filter = Query::<RunsFilterQuery>::try_from_uri(&uri)
            .expect("Failed to deserialize empty query params in RunsFilterQuery")
            .0;
        assert_eq!(filter.automation_id, None);
        assert_eq!(filter.worker_id, None);
        assert_eq!(filter.before_id, None);
        assert_eq!(filter.status.as_deref(), Some(""));

        let uri_valid = format!(
            "/runs?{}",
            "automation_id=10&worker_id=5&status=succeeded&before_id=100"
        )
        .parse::<axum::http::Uri>()
        .expect("Failed to parse URI");
        let filter_valid = Query::<RunsFilterQuery>::try_from_uri(&uri_valid)
            .expect("Failed to deserialize valid query params in RunsFilterQuery")
            .0;
        assert_eq!(filter_valid.automation_id, Some(10));
        assert_eq!(filter_valid.worker_id, Some(5));
        assert_eq!(filter_valid.before_id, Some(100));
        assert_eq!(filter_valid.status.as_deref(), Some("succeeded"));
    }
}
