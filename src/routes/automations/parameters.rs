use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use sqlx::Row;

use super::types::*;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, log_audit};
use crate::routes::auth::HtmlTemplate;

pub fn is_valid_param_type(pt: &str) -> bool {
    matches!(pt, "int" | "bool" | "color" | "string")
}

pub fn validate_parameter_default_value(
    param_type: &str,
    default_value: &str,
) -> Result<(), String> {
    let val = default_value.trim();
    match param_type {
        "int" => {
            if val.parse::<i64>().is_err() {
                return Err(format!("Default value «{}» is not a valid integer.", val));
            }
        }
        "bool" => {
            let lower = val.to_lowercase();
            if !matches!(lower.as_str(), "true" | "false" | "1" | "0" | "yes" | "no") {
                return Err(format!(
                    "Default value «{}» is not a valid boolean (expected true/false, 1/0, yes/no).",
                    val
                ));
            }
        }
        "color" => {
            if val.is_empty() {
                return Err("Default value for color cannot be empty.".to_string());
            }
        }
        "string" => {
            // Any string is allowed
        }
        _ => return Err(format!("Invalid parameter type «{}».", param_type)),
    }
    Ok(())
}

/// GET /automations/{id}/parameters
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_parameters_handler(
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

    let parameters = sqlx::query_as::<_, AutomationParameterItem>(
        "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    HtmlTemplate(AutomationParametersTemplate {
        user,
        csrf_token,
        automation,
        parameters,
        error: None,
    })
    .into_response()
}

/// POST /automations/{id}/parameters
#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_automation_parameter_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<ParameterForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
    }

    let name = form.name.trim();
    let param_type = form.param_type.trim();
    let default_value = form.default_value.trim();
    let description = form.description.unwrap_or_default().trim().to_string();

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

    let mut err = None;
    if name.is_empty() {
        err = Some("Parameter name cannot be empty.".to_string());
    } else if !is_valid_param_type(param_type) {
        err = Some("Invalid parameter type. Must be one of: int, bool, color, string.".to_string());
    } else if let Err(val_err) = validate_parameter_default_value(param_type, default_value) {
        err = Some(val_err);
    }

    if let Some(error_msg) = err {
        let parameters = sqlx::query_as::<_, AutomationParameterItem>(
            "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationParametersTemplate {
                user,
                csrf_token,
                automation,
                parameters,
                error: Some(error_msg),
            }),
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
        }
    };

    let insert_res = sqlx::query(
        "INSERT INTO automation_parameters (automation_id, name, param_type, default_value, description) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(id)
    .bind(name)
    .bind(param_type)
    .bind(default_value)
    .bind(&description)
    .fetch_one(&mut *tx)
    .await;

    let pid: i64 = match insert_res {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to insert automation parameter: {}", e);
            let _ = tx.rollback().await;
            let parameters = sqlx::query_as::<_, AutomationParameterItem>(
                "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

            let err_msg = if e.to_string().contains("unique") || e.to_string().contains("duplicate")
            {
                format!(
                    "A parameter named «{}» already exists in this automation.",
                    name
                )
            } else {
                "Failed to create parameter in database.".to_string()
            };

            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(AutomationParametersTemplate {
                    user,
                    csrf_token,
                    automation,
                    parameters,
                    error: Some(err_msg),
                }),
            )
                .into_response();
        }
    };

    let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await;

    if tx.commit().await.is_ok() {
        tracing::info!(
            "Created automation parameter '{}' for automation {}",
            name,
            id
        );
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "create_parameter",
            "automation_parameters",
            Some(pid),
            Some(serde_json::json!({
                "automation_id": id,
                "name": name,
                "param_type": param_type,
                "default_value": default_value,
            })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}/parameters", id)).into_response()
}

/// POST /automations/{id}/parameters/{pid}
#[tracing::instrument(skip(state, user, form))]
pub async fn post_update_automation_parameter_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, pid)): Path<(i64, i64)>,
    CsrfForm(form): CsrfForm<ParameterForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
    }

    let name = form.name.trim();
    let param_type = form.param_type.trim();
    let default_value = form.default_value.trim();
    let description = form.description.unwrap_or_default().trim().to_string();

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

    let mut err = None;
    if name.is_empty() {
        err = Some("Parameter name cannot be empty.".to_string());
    } else if !is_valid_param_type(param_type) {
        err = Some("Invalid parameter type. Must be one of: int, bool, color, string.".to_string());
    } else if let Err(val_err) = validate_parameter_default_value(param_type, default_value) {
        err = Some(val_err);
    }

    if let Some(error_msg) = err {
        let parameters = sqlx::query_as::<_, AutomationParameterItem>(
            "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationParametersTemplate {
                user,
                csrf_token,
                automation,
                parameters,
                error: Some(error_msg),
            }),
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
        }
    };

    let update_res = sqlx::query(
        "UPDATE automation_parameters SET name = $1, param_type = $2, default_value = $3, description = $4 WHERE id = $5 AND automation_id = $6",
    )
    .bind(name)
    .bind(param_type)
    .bind(default_value)
    .bind(&description)
    .bind(pid)
    .bind(id)
    .execute(&mut *tx)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update automation parameter {}: {}", pid, e);
        let _ = tx.rollback().await;
        let parameters = sqlx::query_as::<_, AutomationParameterItem>(
            "SELECT id, automation_id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let err_msg = if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
            format!(
                "A parameter named «{}» already exists in this automation.",
                name
            )
        } else {
            "Failed to update parameter in database.".to_string()
        };

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationParametersTemplate {
                user,
                csrf_token,
                automation,
                parameters,
                error: Some(err_msg),
            }),
        )
            .into_response();
    }

    let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await;

    if tx.commit().await.is_ok() {
        tracing::info!("Updated automation parameter {} for automation {}", pid, id);
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "update_parameter",
            "automation_parameters",
            Some(pid),
            Some(serde_json::json!({
                "automation_id": id,
                "name": name,
                "param_type": param_type,
                "default_value": default_value,
            })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}/parameters", id)).into_response()
}

/// POST /automations/{id}/parameters/{pid}/delete
#[tracing::instrument(skip(state, user))]
pub async fn post_delete_automation_parameter_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, pid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}/parameters", id)).into_response();
        }
    };

    let delete_res =
        sqlx::query("DELETE FROM automation_parameters WHERE id = $1 AND automation_id = $2")
            .bind(pid)
            .bind(id)
            .execute(&mut *tx)
            .await;

    match delete_res {
        Ok(_) => {
            let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await;

            if tx.commit().await.is_ok() {
                tracing::info!(
                    "Deleted automation parameter {} from automation {}",
                    pid,
                    id
                );
                let _ = log_audit(
                    &state.db,
                    Some(user.id),
                    "delete_parameter",
                    "automation_parameters",
                    Some(pid),
                    Some(serde_json::json!({ "automation_id": id })),
                )
                .await;
            }

            Redirect::to(&format!("/automations/{}/parameters", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete automation parameter {}: {}", pid, e);
            let _ = tx.rollback().await;
            Redirect::to(&format!("/automations/{}/parameters", id)).into_response()
        }
    }
}
