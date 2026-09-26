use askama::Template;
use axum::{extract::State, response::IntoResponse};

use super::auth::HtmlTemplate;
use crate::auth::AuthUser;
use crate::AppState;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub user: AuthUser,
    pub active_automations_count: i64,
    pub workers_online_count: i64,
    pub runs_today_count: i64,
    pub failed_lost_runs_today_count: i64,
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

    HtmlTemplate(IndexTemplate {
        user,
        active_automations_count,
        workers_online_count,
        runs_today_count,
        failed_lost_runs_today_count,
    })
}
