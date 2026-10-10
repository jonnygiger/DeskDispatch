use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use sqlx::Row;

use super::types::*;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, log_audit};
use crate::routes::auth::HtmlTemplate;

/// GET /automations
#[tracing::instrument(skip(state, user))]
pub async fn get_automations_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<AutomationsListQuery>,
) -> impl IntoResponse {
    let status_filter = query.status.unwrap_or_else(|| "all".to_string());
    let search_query = query.q.unwrap_or_default();

    let mut sql = String::from(
        r#"
        SELECT
            a.id,
            a.name,
            a.description,
            a.status,
            COALESCE(s.step_count, 0) AS step_count,
            a.updated_at,
            lr.id AS last_run_id,
            lr.status AS last_run_status,
            lr.queued_at AS last_run_at
        FROM automations a
        LEFT JOIN (
            SELECT automation_id, COUNT(*) AS step_count
            FROM automation_steps
            GROUP BY automation_id
        ) s ON a.id = s.automation_id
        LEFT JOIN LATERAL (
            SELECT id, status, queued_at
            FROM task_runs
            WHERE automation_id = a.id
            ORDER BY queued_at DESC, id DESC
            LIMIT 1
        ) lr ON true
        WHERE 1=1
        "#,
    );

    if status_filter != "all" {
        sql.push_str(&format!(
            " AND a.status = '{}'",
            status_filter.replace('\'', "''")
        ));
    }

    if !search_query.trim().is_empty() {
        let escaped = search_query.trim().replace('\'', "''");
        sql.push_str(&format!(
            " AND (a.name ILIKE '%{}%' OR a.description ILIKE '%{}%')",
            escaped, escaped
        ));
    }

    sql.push_str(" ORDER BY a.updated_at DESC, a.id DESC");

    let automations: Vec<AutomationListItem> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    HtmlTemplate(AutomationsListTemplate {
        user,
        automations,
        status_filter,
        search_query,
    })
}

/// GET /automations/new
#[tracing::instrument(skip(user))]
pub async fn get_new_automation_handler(user: AuthUser) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    HtmlTemplate(AutomationNewTemplate {
        user,
        csrf_token,
        error: None,
        name: String::new(),
        description: String::new(),
    })
}

/// POST /automations
#[tracing::instrument(skip(state, user, form))]
pub async fn post_automations_handler(
    State(state): State<AppState>,
    user: AuthUser,
    CsrfForm(form): CsrfForm<CreateAutomationForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return (
            StatusCode::FORBIDDEN,
            HtmlTemplate(AutomationNewTemplate {
                user,
                csrf_token,
                error: Some("Permission denied.".to_string()),
                name: form.name,
                description: form.description.unwrap_or_default(),
            }),
        )
            .into_response();
    }

    let name = form.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AutomationNewTemplate {
                user,
                csrf_token,
                error: Some("Automation name cannot be empty.".to_string()),
                name: String::new(),
                description: form.description.unwrap_or_default(),
            }),
        )
            .into_response();
    }

    let description = form.description.unwrap_or_default().trim().to_string();

    let record = sqlx::query(
        "INSERT INTO automations (name, description, status, created_by) VALUES ($1, $2, 'draft', $3) RETURNING id",
    )
    .bind(name)
    .bind(&description)
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    match record {
        Ok(row) => {
            let id: i64 = row.get("id");
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "create_automation",
                "automation",
                Some(id),
                Some(serde_json::json!({ "name": name, "description": description })),
            )
            .await;

            Redirect::to(&format!("/automations/{}", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create automation: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(AutomationNewTemplate {
                    user,
                    csrf_token,
                    error: Some("Failed to create automation in database.".to_string()),
                    name: name.to_string(),
                    description,
                }),
            )
                .into_response()
        }
    }
}
