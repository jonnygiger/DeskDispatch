use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use sqlx::Row;

use super::parameters::validate_parameter_default_value;
use super::steps::fetch_automation_step_views;
use super::types::*;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, log_audit};
use crate::routes::auth::HtmlTemplate;

/// GET /automations/{id}
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_detail_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let automation = match sqlx::query_as::<_, AutomationDetail>(
        "SELECT id, name, description, status, created_by, created_at, updated_at FROM automations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return Redirect::to("/automations").into_response(),
        Err(e) => {
            tracing::error!("Error fetching automation detail: {}", e);
            return Redirect::to("/automations").into_response();
        }
    };

    // Optimization: Run step views, worker groups, and parameter option queries concurrently using `tokio::join!`.
    // Steps are bulk-loaded in O(1) queries using `fetch_automation_step_views`, eliminating per-step N+1 database queries.
    let (steps, worker_groups_res, parameters_res) = tokio::join!(
        fetch_automation_step_views(&state.db, id),
        sqlx::query_as::<_, WorkerGroupOption>("SELECT id, name FROM worker_groups ORDER BY name ASC").fetch_all(&state.db),
        sqlx::query_as::<_, AutomationParameterItem>("SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC").bind(id).fetch_all(&state.db)
    );
    let worker_groups = worker_groups_res.unwrap_or_default();
    let parameters = parameters_res.unwrap_or_default();

    HtmlTemplate(AutomationDetailTemplate {
        user,
        csrf_token,
        automation,
        steps,
        worker_groups,
        parameters,
        active_tab: "steps".to_string(),
        error: None,
        success: None,
    })
    .into_response()
}

/// POST /automations/{id}
#[tracing::instrument(skip(state, user, form))]
pub async fn post_automation_edit_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<EditAutomationForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let name = form.name.trim();
    if name.is_empty() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let description = form.description.unwrap_or_default().trim().to_string();
    let requested_status = form.status.as_str();

    let final_status = if requested_status == "active" {
        if let Err(lint_errors) =
            crate::validation::validate_automation_for_activation(&state.db, id).await
        {
            let error_msgs: Vec<String> = lint_errors.iter().map(|e| e.to_string()).collect();
            let combined_err = format!(
                "Cannot activate automation due to validation errors: {}",
                error_msgs.join("; ")
            );
            tracing::warn!(automation_id = id, errors = %combined_err, "Refusing activation due to lint errors");

            let flash = crate::auth::FlashMessage::error(combined_err);
            let (c1, c2) = crate::auth::build_flash_cookie(&flash);
            return (
                [
                    (axum::http::header::SET_COOKIE, c1),
                    (axum::http::header::SET_COOKIE, c2),
                ],
                Redirect::to(&format!("/automations/{}", id)),
            )
                .into_response();
        }
        "active".to_string()
    } else {
        match requested_status {
            "archived" | "draft" => requested_status.to_string(),
            _ => "draft".to_string(),
        }
    };

    let res = sqlx::query(
        "UPDATE automations SET name = $1, description = $2, status = $3, updated_at = now() WHERE id = $4",
    )
    .bind(name)
    .bind(&description)
    .bind(&final_status)
    .bind(id)
    .execute(&state.db)
    .await;

    if res.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "update_automation",
            "automation",
            Some(id),
            Some(serde_json::json!({ "name": name, "description": description, "status": final_status })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// GET /automations/{id}/delete
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_delete_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let automation = match sqlx::query_as::<_, AutomationDetail>(
        "SELECT id, name, description, status, created_by, created_at, updated_at FROM automations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(a)) => a,
        _ => return Redirect::to("/automations").into_response(),
    };

    HtmlTemplate(AutomationDeleteTemplate {
        user,
        csrf_token,
        automation,
    })
    .into_response()
}

/// POST /automations/{id}/delete
#[tracing::instrument(skip(state, user))]
pub async fn post_automation_delete_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to("/automations").into_response();
    }

    // Check if automation has execution history in task_runs
    let has_history: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_runs WHERE automation_id = $1)")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);

    if has_history {
        // Archive the automation to preserve execution history
        let res = sqlx::query(
            "UPDATE automations SET status = 'archived', updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(&state.db)
        .await;

        if res.is_ok() {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "archive_automation",
                "automation",
                Some(id),
                Some(serde_json::json!({ "reason": "has_task_runs" })),
            )
            .await;
        }
    } else {
        let res = sqlx::query("DELETE FROM automations WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await;

        if res.is_ok() {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "delete_automation",
                "automation",
                Some(id),
                None,
            )
            .await;
        }
    }

    Redirect::to("/automations").into_response()
}

/// POST /automations/{id}/run-now
#[tracing::instrument(skip(state, user))]
pub async fn post_run_now_automation_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<RunNowForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let auto_row = match sqlx::query("SELECT status FROM automations WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => row,
        _ => return Redirect::to("/automations").into_response(),
    };

    let status: String = auto_row.get("status");

    // Refuse draft or archived automations
    if status != "active" {
        tracing::warn!(
            automation_id = id,
            status = %status,
            "Refusing Run Now trigger for non-active automation"
        );
        let flash = crate::auth::FlashMessage::error("Cannot run non-active automation.");
        let (c1, c2) = crate::auth::build_flash_cookie(&flash);
        return (
            [
                (axum::http::header::SET_COOKIE, c1),
                (axum::http::header::SET_COOKIE, c2),
            ],
            Redirect::to(&format!("/automations/{}", id)),
        )
            .into_response();
    }

    if let Err(lint_errors) =
        crate::validation::validate_automation_for_activation(&state.db, id).await
    {
        let error_msgs: Vec<String> = lint_errors.iter().map(|e| e.to_string()).collect();
        let combined_err = format!(
            "Cannot run automation due to validation errors: {}",
            error_msgs.join("; ")
        );
        tracing::warn!(automation_id = id, errors = %combined_err, "Refusing Run Now trigger due to lint errors");

        let flash = crate::auth::FlashMessage::error(combined_err);
        let (c1, c2) = crate::auth::build_flash_cookie(&flash);
        return (
            [
                (axum::http::header::SET_COOKIE, c1),
                (axum::http::header::SET_COOKIE, c2),
            ],
            Redirect::to(&format!("/automations/{}", id)),
        )
            .into_response();
    }

    // Refuse empty automations (automations without any active steps)
    let step_count: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    {
        Ok(cnt) => cnt,
        Err(e) => {
            tracing::error!(automation_id = id, "Error checking step count: {}", e);
            let flash = crate::auth::FlashMessage::error("Error checking automation steps.");
            let (c1, c2) = crate::auth::build_flash_cookie(&flash);
            return (
                [
                    (axum::http::header::SET_COOKIE, c1),
                    (axum::http::header::SET_COOKIE, c2),
                ],
                Redirect::to(&format!("/automations/{}", id)),
            )
                .into_response();
        }
    };

    if step_count == 0 {
        tracing::warn!(
            automation_id = id,
            "Refusing Run Now trigger for automation with no active steps"
        );
        let flash = crate::auth::FlashMessage::error("Cannot run automation with no steps.");
        let (c1, c2) = crate::auth::build_flash_cookie(&flash);
        return (
            [
                (axum::http::header::SET_COOKIE, c1),
                (axum::http::header::SET_COOKIE, c2),
            ],
            Redirect::to(&format!("/automations/{}", id)),
        )
            .into_response();
    }

    let target_worker_group_id = form.worker_group_id;

    // Fetch defined parameters for type validation & filtering overrides
    let defined_params = sqlx::query_as::<_, AutomationParameterItem>(
        "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut clean_overrides = std::collections::HashMap::new();
    for param in &defined_params {
        if let Some(val) = form.parameters.get(&param.name) {
            let val_trimmed = val.trim();
            if !val_trimmed.is_empty() {
                if let Err(e) = validate_parameter_default_value(&param.param_type, val_trimmed) {
                    tracing::warn!(automation_id = id, error = %e, "Invalid parameter override value");
                    let flash = crate::auth::FlashMessage::error(e);
                    let (c1, c2) = crate::auth::build_flash_cookie(&flash);
                    return (
                        [
                            (axum::http::header::SET_COOKIE, c1),
                            (axum::http::header::SET_COOKIE, c2),
                        ],
                        Redirect::to(&format!("/automations/{}", id)),
                    )
                        .into_response();
                }
                clean_overrides.insert(param.name.clone(), val_trimmed.to_string());
            }
        }
    }

    let parameter_overrides_json = if clean_overrides.is_empty() {
        None
    } else {
        Some(serde_json::json!(clean_overrides))
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for run now: {}", e);
            let flash = crate::auth::FlashMessage::error("Database error.");
            let (c1, c2) = crate::auth::build_flash_cookie(&flash);
            return (
                [
                    (axum::http::header::SET_COOKIE, c1),
                    (axum::http::header::SET_COOKIE, c2),
                ],
                Redirect::to(&format!("/automations/{}", id)),
            )
                .into_response();
        }
    };

    let run_res = sqlx::query(
        r#"
        INSERT INTO task_runs (automation_id, schedule_id, worker_id, target_worker_group_id, parameter_overrides, status, triggered_by_user_id, queued_at)
        VALUES ($1, NULL, NULL, $2, $3, 'queued', $4, now())
        RETURNING id
        "#,
    )
    .bind(id)
    .bind(target_worker_group_id)
    .bind(&parameter_overrides_json)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await;

    let task_run_id: i64 = match run_res {
        Ok(row) => row.get("id"),
        Err(e) => {
            tracing::error!(
                "Failed to queue manual task run for automation {}: {}",
                id,
                e
            );
            let _ = tx.rollback().await;
            let flash = crate::auth::FlashMessage::error("Failed to queue run.");
            let (c1, c2) = crate::auth::build_flash_cookie(&flash);
            return (
                [
                    (axum::http::header::SET_COOKIE, c1),
                    (axum::http::header::SET_COOKIE, c2),
                ],
                Redirect::to(&format!("/automations/{}", id)),
            )
                .into_response();
        }
    };

    // Pre-snapshot full automation payload incorporating parameter overrides
    let overrides_ref = if clean_overrides.is_empty() {
        None
    } else {
        Some(&clean_overrides)
    };
    if let Ok(automation_json) =
        crate::routes::api_workers::fetch_full_automation_json_with_overrides(
            &mut tx,
            id,
            overrides_ref,
        )
        .await
    {
        let _ = sqlx::query("UPDATE task_runs SET dispatched_automation_json = $1 WHERE id = $2")
            .bind(&automation_json)
            .bind(task_run_id)
            .execute(&mut *tx)
            .await;
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(
            "Failed to commit transaction for task run {}: {}",
            task_run_id,
            e
        );
    }

    tracing::info!(
        task_run_id = task_run_id,
        automation_id = id,
        target_worker_group_id = ?target_worker_group_id,
        "Manually queued task run for automation"
    );

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "trigger_run_now",
        "task_run",
        Some(task_run_id),
        Some(serde_json::json!({
            "automation_id": id,
            "target_worker_group_id": target_worker_group_id,
            "parameter_overrides": clean_overrides,
        })),
    )
    .await;

    let flash =
        crate::auth::FlashMessage::success(format!("Run #{} queued successfully.", task_run_id));
    let (c1, c2) = crate::auth::build_flash_cookie(&flash);
    (
        [
            (axum::http::header::SET_COOKIE, c1),
            (axum::http::header::SET_COOKIE, c2),
        ],
        Redirect::to(&format!("/runs/{}", task_run_id)),
    )
        .into_response()
}
