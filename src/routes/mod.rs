pub mod account;
pub mod api_workers;
pub mod auth;
pub mod automations;
pub mod bitmaps;
pub mod errors;
pub mod home;
pub mod media;
pub mod recordings;
pub mod runs;
pub mod schedules;
pub mod static_assets;
pub mod deletion_tests;
pub mod users;
pub mod workers;

pub use account::{get_password_handler, post_password_handler};
pub use api_workers::{
    fetch_full_automation_json, fetch_full_automation_json_with_overrides,
    get_next_assignment_handler, get_recording_screenshot_upload_url_handler,
    get_task_run_handler, get_task_run_screenshot_upload_url_handler,
    is_agent_version_outdated, is_valid_step_result, is_valid_worker_status,
    post_complete_task_run_handler, post_heartbeat_handler, post_recording_events_handler,
    post_register_worker_handler, post_step_result_handler,
    post_task_run_screenshot_commit_handler, post_worker_stop_recording_handler,
    sweep_stalled_task_runs,
};
pub use auth::{get_login_handler, post_login_handler, post_logout_handler};
pub use automations::*;
pub use bitmaps::*;
pub use errors::{
    internal_error_handler, not_found_handler, InternalServerErrorTemplate, NotFoundTemplate,
};
pub use home::get_index_handler;
pub use media::*;
pub use recordings::*;
pub use runs::*;
pub use schedules::*;
pub use static_assets::static_asset_handler;
pub use users::*;
pub use workers::{
    get_edit_worker_group_handler, get_edit_worker_handler,
    get_new_worker_group_handler, get_new_worker_handler, get_worker_detail_handler,
    get_workers_handler, post_create_worker_group_handler, post_create_worker_handler,
    post_deactivate_worker_handler, post_delete_worker_group_handler, post_delete_worker_handler,
    post_edit_worker_group_handler, post_edit_worker_handler, post_rotate_worker_key_handler,
    WorkerDetail, WorkerGroupItem, WorkerPcItem,
};

pub async fn livez_handler() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, "OK")
}

pub async fn readyz_handler(axum::extract::State(state): axum::extract::State<crate::AppState>) -> impl axum::response::IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (axum::http::StatusCode::OK, "OK"),
        Err(err) => {
            tracing::error!("Readyz check failed: {}", err);
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Database connection error")
        }
    }
}
