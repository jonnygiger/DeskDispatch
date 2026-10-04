use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use sqlx::Row;
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, AuthUser};
use crate::magnifier::ImageMagnifier;
use crate::AppState;

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RunsFilterQuery {
    pub automation_id: Option<i64>,
    pub worker_id: Option<i64>,
    pub status: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CancelRunForm {
    pub csrf_token: String,
}

#[derive(Debug, Clone)]
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
            "succeeded" => "badge-success",
            "failed" => "badge-danger",
            "cancelled" => "badge-secondary",
            "lost" => "badge-danger",
            _ => "badge-secondary",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutomationOption {
    pub id: i64,
    pub name: String,
}

impl AutomationOption {
    pub fn is_selected(&self, filter_id: &Option<i64>) -> bool {
        *filter_id == Some(self.id)
    }
}

#[derive(Debug, Clone)]
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

#[derive(Template)]
#[template(path = "runs/detail.html")]
pub struct RunDetailTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub run: TaskRunListItem,
    pub steps: Vec<ExecutedStepItem>,
    pub variable_values: Vec<RunVariableValueItem>,
    pub auto_refresh: bool,
}

pub async fn get_runs_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Query(filter): Query<RunsFilterQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let auto_rows = sqlx::query("SELECT id, name FROM automations ORDER BY name ASC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let automations = auto_rows
        .into_iter()
        .map(|r| AutomationOption {
            id: r.get("id"),
            name: r.get("name"),
        })
        .collect();

    let worker_rows = sqlx::query("SELECT id, display_name FROM task_worker_pcs ORDER BY display_name ASC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let workers = worker_rows
        .into_iter()
        .map(|r| WorkerOption {
            id: r.get("id"),
            display_name: r.get("display_name"),
        })
        .collect();

    let filter_status_clean = filter
        .status
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "all");

    let run_rows = sqlx::query(
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
            tr.error_message
        FROM task_runs tr
        JOIN automations a ON tr.automation_id = a.id
        LEFT JOIN schedules s ON tr.schedule_id = s.id
        LEFT JOIN task_worker_pcs w ON tr.worker_id = w.id
        LEFT JOIN users u ON tr.triggered_by_user_id = u.id
        WHERE ($1::BIGINT IS NULL OR tr.automation_id = $1)
          AND ($2::BIGINT IS NULL OR tr.worker_id = $2)
          AND ($3::TEXT IS NULL OR tr.status = $3)
        ORDER BY tr.queued_at DESC, tr.id DESC
        LIMIT 100
        "#,
    )
    .bind(filter.automation_id)
    .bind(filter.worker_id)
    .bind(filter_status_clean.as_deref())
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let runs = run_rows
        .into_iter()
        .map(|r| TaskRunListItem {
            id: r.get("id"),
            automation_id: r.get("automation_id"),
            automation_name: r.get("automation_name"),
            schedule_id: r.get("schedule_id"),
            schedule_name: r.get("schedule_name"),
            worker_id: r.get("worker_id"),
            worker_name: r.get("worker_name"),
            status: r.get("status"),
            triggered_by: r.get("triggered_by"),
            queued_at: r.get("queued_at"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            error_message: r.get("error_message"),
        })
        .collect();

    HtmlTemplate(RunsListTemplate {
        user,
        csrf_token,
        runs,
        automations,
        workers,
        filter_automation_id: filter.automation_id,
        filter_worker_id: filter.worker_id,
        filter_status: filter_status_clean,
    })
}

pub async fn get_run_detail_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let run_row = match sqlx::query(
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
            tr.error_message
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
        _ => return Redirect::to("/runs").into_response(),
    };

    let worker_screen_width: u32 = run_row
        .get::<Option<i32>, _>("worker_screen_width")
        .unwrap_or(1920)
        .max(1) as u32;
    let worker_screen_height: u32 = run_row
        .get::<Option<i32>, _>("worker_screen_height")
        .unwrap_or(1080)
        .max(1) as u32;

    let run_item = TaskRunListItem {
        id: run_row.get("id"),
        automation_id: run_row.get("automation_id"),
        automation_name: run_row.get("automation_name"),
        schedule_id: run_row.get("schedule_id"),
        schedule_name: run_row.get("schedule_name"),
        worker_id: run_row.get("worker_id"),
        worker_name: run_row.get("worker_name"),
        status: run_row.get("status"),
        triggered_by: run_row.get("triggered_by"),
        queued_at: run_row.get("queued_at"),
        started_at: run_row.get("started_at"),
        completed_at: run_row.get("completed_at"),
        error_message: run_row.get("error_message"),
    };

    let auto_refresh = run_item.status == "queued" || run_item.status == "running";

    let step_rows = sqlx::query(
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
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut executed_steps = Vec::new();
    for (idx, r) in step_rows.into_iter().enumerate() {
        let step_id: i64 = r.get("step_id");
        let screenshot_key: Option<String> = r
            .get::<Option<String>, _>("screenshot_object_key")
            .or_else(|| r.get("step_screenshot_key"));

        let captured_x: Option<i32> = r.get("captured_x");
        let captured_y: Option<i32> = r.get("captured_y");

        let target_x = captured_x.map(|x| x.max(0) as u32);
        let target_y = captured_y.map(|y| y.max(0) as u32);

        let magnifier = if let Some(ref key) = screenshot_key {
            let presigned_url = match state
                .storage_service()
                .generate_presigned_get_url(key, Duration::from_secs(900))
                .await
            {
                Ok(url) => url,
                Err(_) => format!("/media/screenshots/{}", step_id),
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
            id: r.get("id"),
            step_id,
            step_number: idx + 1,
            step_type: r.get("step_type"),
            label: r.get("label"),
            result: r.get("result"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            captured_r: r.get("captured_r"),
            captured_g: r.get("captured_g"),
            captured_b: r.get("captured_b"),
            captured_found: r.get("captured_found"),
            captured_x,
            captured_y,
            screenshot_object_key: screenshot_key,
            magnifier,
        });
    }

    let var_rows = sqlx::query(
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
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let variable_values = var_rows
        .into_iter()
        .map(|r| RunVariableValueItem {
            variable_id: r.get("variable_id"),
            variable_name: r.get("variable_name"),
            var_type: r.get("var_type"),
            value: r.get("value"),
            set_at_step_id: r.get("set_at_step_id"),
            set_at: r.get("set_at"),
        })
        .collect();

    HtmlTemplate(RunDetailTemplate {
        user,
        csrf_token,
        run: run_item,
        steps: executed_steps,
        variable_values,
        auto_refresh,
    })
    .into_response()
}

pub async fn post_cancel_run_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<CancelRunForm>,
) -> Response {
    if !user.role.can_edit() {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot cancel task runs").into_response();
    }
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let update_res = sqlx::query(
        r#"
        UPDATE task_runs
        SET status = 'cancelled',
            completed_at = now()
        WHERE id = $1 AND status IN ('queued', 'running')
        RETURNING automation_id, worker_id
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await;

    match update_res {
        Ok(Some(row)) => {
            let automation_id: i64 = row.get("automation_id");
            let worker_id: Option<i64> = row.get("worker_id");

            let details = serde_json::json!({
                "automation_id": automation_id,
                "worker_id": worker_id,
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
mod tests {
    use super::*;

    #[test]
    fn test_task_run_list_item_methods() {
        let now = chrono::Utc::now();
        let item = TaskRunListItem {
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
        };

        assert_eq!(item.status_badge_class(), "badge-success");
        assert_eq!(item.duration_display(), "45s");
        assert!(!item.formatted_queued_at().is_empty());
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
        assert_eq!(step_item.captured_rgb_display(), Some("RGB(40, 180, 60)".to_string()));
        assert_eq!(step_item.captured_xy_display(), Some("(824, 391)".to_string()));
    }
}
