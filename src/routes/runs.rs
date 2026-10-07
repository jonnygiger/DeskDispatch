use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use sqlx::Row;
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, AuthUser, CsrfForm};
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
            "cancelling" => "badge-warning",
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
        sqlx::query("SELECT id, name FROM automations ORDER BY name ASC").fetch_all(&state.db),
        sqlx::query("SELECT id, display_name FROM task_worker_pcs ORDER BY display_name ASC").fetch_all(&state.db),
        sqlx::query(
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
    );

    let auto_rows = auto_rows_res.unwrap_or_default();
    let automations = auto_rows
        .into_iter()
        .map(|r| AutomationOption {
            id: r.get("id"),
            name: r.get("name"),
        })
        .collect();

    let worker_rows = worker_rows_res.unwrap_or_default();
    let workers = worker_rows
        .into_iter()
        .map(|r| WorkerOption {
            id: r.get("id"),
            display_name: r.get("display_name"),
        })
        .collect();

    let run_rows = run_rows_res.unwrap_or_default();

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

#[tracing::instrument(skip(state, user))]
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

    let auto_refresh = run_item.status == "queued" || run_item.status == "running" || run_item.status == "cancelling";

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

#[tracing::instrument(skip(state, user, _form))]
pub async fn post_cancel_run_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(_form): CsrfForm<CancelRunForm>,
) -> Response {
    if !user.role.can_edit() {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot cancel task runs").into_response();
    }

    let update_res = sqlx::query(
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
        assert_eq!(step_item.captured_rgb_display(), Some("RGB(40, 180, 60)".to_string()));
        assert_eq!(step_item.captured_xy_display(), Some("(824, 391)".to_string()));
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
            assert_eq!(item.status_badge_class(), expected_class, "Status '{}' mismatch", status);
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
        assert_eq!(step.captured_xy_display(), None);  // Incomplete XY

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
        let auto_opt = AutomationOption { id: 42, name: "Auto 42".to_string() };
        assert!(auto_opt.is_selected(&Some(42)));
        assert!(!auto_opt.is_selected(&Some(10)));
        assert!(!auto_opt.is_selected(&None));

        let worker_opt = WorkerOption { id: 7, display_name: "Worker 7".to_string() };
        assert!(worker_opt.is_selected(&Some(7)));
        assert!(!worker_opt.is_selected(&Some(1)));
        assert!(!worker_opt.is_selected(&None));
    }
}
