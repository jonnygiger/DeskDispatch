use askama::Template;
use axum::{extract::State, response::IntoResponse};
use chrono::{DateTime, Utc};

use super::auth::HtmlTemplate;
use crate::auth::AuthUser;
use crate::AppState;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecentRunItem {
    pub id: i64,
    pub automation_id: i64,
    pub automation_name: String,
    pub worker_id: Option<i64>,
    pub worker_name: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub queued_at: DateTime<Utc>,
}

impl RecentRunItem {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "succeeded" => "badge-success",
            "failed" | "lost" => "badge-danger",
            "running" => "badge-warning",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_queued_at(&self) -> String {
        self.queued_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub user: AuthUser,
    pub active_automations_count: i64,
    pub workers_online_count: i64,
    pub runs_today_count: i64,
    pub failed_lost_runs_today_count: i64,
    pub recent_runs: Vec<RecentRunItem>,
}

pub async fn get_index_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    let active_automations_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM automations WHERE status = 'active'")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let workers_online_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_worker_pcs WHERE status = 'online'")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let runs_today_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_runs WHERE queued_at >= CURRENT_DATE")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let failed_lost_runs_today_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_runs WHERE status IN ('failed', 'lost') AND queued_at >= CURRENT_DATE",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let recent_runs: Vec<RecentRunItem> = sqlx::query_as(
        r#"
        SELECT
            tr.id,
            tr.automation_id,
            a.name AS automation_name,
            tr.worker_id,
            w.display_name AS worker_name,
            tr.status,
            COALESCE(u.display_name, u.username, s.name, 'Manual') AS triggered_by,
            tr.queued_at
        FROM task_runs tr
        JOIN automations a ON tr.automation_id = a.id
        LEFT JOIN task_worker_pcs w ON tr.worker_id = w.id
        LEFT JOIN users u ON tr.triggered_by_user_id = u.id
        LEFT JOIN schedules s ON tr.schedule_id = s.id
        ORDER BY tr.queued_at DESC, tr.id DESC
        LIMIT 10
        "#,
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    HtmlTemplate(IndexTemplate {
        user,
        active_automations_count,
        workers_online_count,
        runs_today_count,
        failed_lost_runs_today_count,
        recent_runs,
    })
}
