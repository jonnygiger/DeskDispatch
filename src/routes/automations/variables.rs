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

pub fn is_valid_var_type(vt: &str) -> bool {
    matches!(vt, "int" | "bool" | "color" | "point" | "string")
}

pub async fn fetch_automation_variables(
    db: &sqlx::PgPool,
    automation_id: i64,
) -> Vec<VariableOption> {
    sqlx::query_as::<_, VariableOption>(
        "SELECT id, name, var_type FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// GET /automations/{id}/variables
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_variables_handler(
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

    let variables = sqlx::query_as::<_, AutomationVariableItem>(
        "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    HtmlTemplate(AutomationVariablesTemplate {
        user,
        csrf_token,
        automation,
        variables,
        error: None,
    })
    .into_response()
}

/// POST /automations/{id}/variables
#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_automation_variable_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<VariableForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/variables", id)).into_response();
    }

    let name = form.name.trim();
    let var_type = form.var_type.trim();
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
        err = Some("Variable name cannot be empty.".to_string());
    } else if !is_valid_var_type(var_type) {
        err = Some(
            "Invalid variable type. Must be one of: int, bool, color, point, string.".to_string(),
        );
    }

    if let Some(error_msg) = err {
        let variables = sqlx::query_as::<_, AutomationVariableItem>(
            "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationVariablesTemplate {
                user,
                csrf_token,
                automation,
                variables,
                error: Some(error_msg),
            }),
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}/variables", id)).into_response();
        }
    };

    let insert_res = sqlx::query(
        "INSERT INTO automation_variables (automation_id, name, var_type, description) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(id)
    .bind(name)
    .bind(var_type)
    .bind(&description)
    .fetch_one(&mut *tx)
    .await;

    let var_id: i64 = match insert_res {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to insert automation variable: {}", e);
            let _ = tx.rollback().await;
            let variables = sqlx::query_as::<_, AutomationVariableItem>(
                "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

            let err_msg = if e.to_string().contains("unique") || e.to_string().contains("duplicate")
            {
                format!(
                    "A variable named «{}» already exists in this automation.",
                    name
                )
            } else {
                "Failed to create variable in database.".to_string()
            };

            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(AutomationVariablesTemplate {
                    user,
                    csrf_token,
                    automation,
                    variables,
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
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "create_variable",
            "automation_variable",
            Some(var_id),
            Some(serde_json::json!({
                "automation_id": id,
                "name": name,
                "var_type": var_type,
                "description": description
            })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}/variables", id)).into_response()
}

/// POST /automations/{id}/variables/{vid}
#[tracing::instrument(skip(state, user, form))]
pub async fn post_update_automation_variable_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, vid)): Path<(i64, i64)>,
    CsrfForm(form): CsrfForm<VariableForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/variables", id)).into_response();
    }

    let name = form.name.trim();
    let var_type = form.var_type.trim();
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
        err = Some("Variable name cannot be empty.".to_string());
    } else if !is_valid_var_type(var_type) {
        err = Some(
            "Invalid variable type. Must be one of: int, bool, color, point, string.".to_string(),
        );
    }

    if let Some(error_msg) = err {
        let variables = sqlx::query_as::<_, AutomationVariableItem>(
            "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationVariablesTemplate {
                user,
                csrf_token,
                automation,
                variables,
                error: Some(error_msg),
            }),
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}/variables", id)).into_response();
        }
    };

    let update_res = sqlx::query(
        "UPDATE automation_variables SET name = $1, var_type = $2, description = $3 WHERE id = $4 AND automation_id = $5",
    )
    .bind(name)
    .bind(var_type)
    .bind(&description)
    .bind(vid)
    .bind(id)
    .execute(&mut *tx)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update automation variable: {}", e);
        let _ = tx.rollback().await;
        let variables = sqlx::query_as::<_, AutomationVariableItem>(
            "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let err_msg = if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
            format!(
                "A variable named «{}» already exists in this automation.",
                name
            )
        } else {
            "Failed to update variable in database.".to_string()
        };

        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationVariablesTemplate {
                user,
                csrf_token,
                automation,
                variables,
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
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "update_variable",
            "automation_variable",
            Some(vid),
            Some(serde_json::json!({
                "automation_id": id,
                "name": name,
                "var_type": var_type,
                "description": description
            })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}/variables", id)).into_response()
}

/// POST /automations/{id}/variables/{vid}/delete
#[tracing::instrument(skip(state, user))]
pub async fn post_delete_automation_variable_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, vid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}/variables", id)).into_response();
    }

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

    let delete_res =
        sqlx::query("DELETE FROM automation_variables WHERE id = $1 AND automation_id = $2")
            .bind(vid)
            .bind(id)
            .execute(&state.db)
            .await;

    match delete_res {
        Ok(_) => {
            let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
                .bind(id)
                .execute(&state.db)
                .await;

            let _ = log_audit(
                &state.db,
                Some(user.id),
                "delete_variable",
                "automation_variable",
                Some(vid),
                Some(serde_json::json!({ "automation_id": id })),
            )
            .await;

            Redirect::to(&format!("/automations/{}/variables", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete automation variable: {}", e);
            let variables = sqlx::query_as::<_, AutomationVariableItem>(
                "SELECT id, automation_id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC, id ASC",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

            let err_msg = if e.to_string().contains("foreign key")
                || e.to_string().contains("violates foreign key constraint")
            {
                "Cannot delete variable because it is currently referenced by one or more automation steps.".to_string()
            } else {
                "Failed to delete variable from database.".to_string()
            };

            (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(AutomationVariablesTemplate {
                    user,
                    csrf_token,
                    automation,
                    variables,
                    error: Some(err_msg),
                }),
            )
                .into_response()
        }
    }
}
