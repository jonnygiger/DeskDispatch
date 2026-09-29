use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use serde::Deserialize;
use sqlx::{PgPool, Row};

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, AuthUser};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct AutomationsListQuery {
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AutomationListItem {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub step_count: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub last_run_id: Option<i64>,
    pub last_run_status: Option<String>,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AutomationListItem {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "active" => "badge-success",
            "archived" => "badge-neutral",
            "draft" => "badge-warning",
            _ => "badge-neutral",
        }
    }

    pub fn last_run_status_badge_class(&self) -> &'static str {
        match self.last_run_status.as_deref() {
            Some("succeeded") => "badge-success",
            Some("failed") | Some("lost") => "badge-danger",
            Some("running") => "badge-warning",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_updated_at(&self) -> String {
        self.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    pub fn formatted_last_run_at(&self) -> String {
        match self.last_run_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "-".to_string(),
        }
    }
}

#[derive(Template)]
#[template(path = "automations/list.html")]
pub struct AutomationsListTemplate {
    pub user: AuthUser,
    pub automations: Vec<AutomationListItem>,
    pub status_filter: String,
    pub search_query: String,
}

#[derive(Template)]
#[template(path = "automations/new.html")]
pub struct AutomationNewTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub error: Option<String>,
    pub name: String,
    pub description: String,
}

#[derive(Deserialize)]
pub struct CreateAutomationForm {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AutomationDetail {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub created_by: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl AutomationDetail {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "active" => "badge-success",
            "archived" => "badge-neutral",
            "draft" => "badge-warning",
            _ => "badge-neutral",
        }
    }
}

#[derive(Debug)]
pub struct StepViewItem {
    pub id: i64,
    pub step_number: usize,
    pub step_type: String,
    pub label: Option<String>,
    pub post_delay_ms: i32,
    pub position: f64,
    pub description: String,
}

impl StepViewItem {
    pub fn formatted_post_delay(&self) -> String {
        if self.post_delay_ms > 0 {
            format!(" · then wait {:.1}s", self.post_delay_ms as f64 / 1000.0)
        } else {
            String::new()
        }
    }
}

#[derive(Template)]
#[template(path = "automations/detail.html")]
pub struct AutomationDetailTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
    pub steps: Vec<StepViewItem>,
    pub active_tab: String,
    pub error: Option<String>,
    pub success: Option<String>,
}

#[derive(Deserialize)]
pub struct EditAutomationForm {
    pub name: String,
    pub description: Option<String>,
    pub status: String,
}

#[derive(Template)]
#[template(path = "automations/delete.html")]
pub struct AutomationDeleteTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
}

#[derive(Template)]
#[template(path = "automations/step_type_picker.html")]
pub struct StepTypePickerTemplate {
    pub user: AuthUser,
    pub automation_id: i64,
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct VariableOption {
    pub id: i64,
    pub name: String,
    pub var_type: String,
}

impl VariableOption {
    pub fn is_selected_x(&self, x_var_id: &Option<i64>) -> bool {
        *x_var_id == Some(self.id)
    }

    pub fn is_selected_y(&self, y_var_id: &Option<i64>) -> bool {
        *y_var_id == Some(self.id)
    }
}

#[derive(Debug, Deserialize)]
pub struct NewMouseClickQuery {
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Template)]
#[template(path = "automations/step_mouse_click.html")]
pub struct StepMouseClickTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub x_mode: String,
    pub x: Option<i32>,
    pub x_variable_id: Option<i64>,
    pub y_mode: String,
    pub y: Option<i32>,
    pub y_variable_id: Option<i64>,
    pub button: String,
    pub click_type: String,
    pub variables: Vec<VariableOption>,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Template)]
#[template(path = "automations/step_key_press.html")]
pub struct StepKeyPressTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub key_combo: String,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Deserialize)]
pub struct KeyPressStepForm {
    pub label: Option<String>,
    pub post_delay_seconds: Option<f64>,
    pub key_combo: String,
}

#[derive(Deserialize)]
pub struct MouseClickStepForm {
    pub step_type: Option<String>,
    pub label: Option<String>,
    pub post_delay_seconds: Option<f64>,
    pub x_mode: Option<String>,
    pub x: Option<i32>,
    pub x_variable_id: Option<i64>,
    pub y_mode: Option<String>,
    pub y: Option<i32>,
    pub y_variable_id: Option<i64>,
    pub button: Option<String>,
    pub click_type: Option<String>,
    pub key_combo: Option<String>,
}

pub async fn fetch_automation_variables(db: &PgPool, automation_id: i64) -> Vec<VariableOption> {
    sqlx::query_as::<_, VariableOption>(
        "SELECT id, name, var_type FROM automation_variables WHERE automation_id = $1 ORDER BY name ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// GET /automations
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
        sql.push_str(&format!(" AND a.status = '{}'", status_filter.replace('\'', "''")));
    }

    if !search_query.trim().is_empty() {
        let escaped = search_query.trim().replace('\'', "''");
        sql.push_str(&format!(" AND (a.name ILIKE '%{}%' OR a.description ILIKE '%{}%')", escaped, escaped));
    }

    sql.push_str(" ORDER BY a.updated_at DESC, a.id DESC");

    let automations: Vec<AutomationListItem> = sqlx::query_as(&sql)
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
pub async fn post_automations_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<CreateAutomationForm>,
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

/// GET /automations/{id}
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

    let raw_steps = match sqlx::query(
        "SELECT id, step_type, label, post_delay_ms, position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching steps: {}", e);
            Vec::new()
        }
    };

    let mut steps = Vec::new();
    for (idx, row) in raw_steps.iter().enumerate() {
        let step_id: i64 = row.get("id");
        let step_type: String = row.get("step_type");
        let label: Option<String> = row.get("label");
        let post_delay_ms: i32 = row.get("post_delay_ms");
        let position: f64 = row.get("position");

        let description = fetch_step_description(&state.db, step_id, &step_type).await;

        steps.push(StepViewItem {
            id: step_id,
            step_number: idx + 1,
            step_type,
            label,
            post_delay_ms,
            position,
            description,
        });
    }

    HtmlTemplate(AutomationDetailTemplate {
        user,
        csrf_token,
        automation,
        steps,
        active_tab: "steps".to_string(),
        error: None,
        success: None,
    })
    .into_response()
}

/// Helper to generate plain-language description for a step
async fn fetch_step_description(db: &PgPool, step_id: i64, step_type: &str) -> String {
    match step_type {
        "key_press" => {
            let row = sqlx::query("SELECT key_combo FROM step_key_presses WHERE step_id = $1")
                .bind(step_id)
                .fetch_optional(db)
                .await;
            if let Ok(Some(r)) = row {
                let combo: String = r.get("key_combo");
                format!("Press {}", combo)
            } else {
                "Press key".to_string()
            }
        }
        "mouse_click" => {
            let row = sqlx::query("SELECT x, y, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
                .bind(step_id)
                .fetch_optional(db)
                .await;
            if let Ok(Some(r)) = row {
                let x: Option<i32> = r.get("x");
                let y: Option<i32> = r.get("y");
                let button: String = r.get("button");
                let click_type: String = r.get("click_type");
                let coord_str = match (x, y) {
                    (Some(x), Some(y)) => format!("({}, {})", x, y),
                    _ => "(variable)".to_string(),
                };
                format!("Click {} [{}, {}]", coord_str, button, click_type)
            } else {
                "Click mouse".to_string()
            }
        }
        "find_pixel_rgb" => {
            let row = sqlx::query("SELECT x, y FROM step_find_pixel_rgb WHERE step_id = $1")
                .bind(step_id)
                .fetch_optional(db)
                .await;
            if let Ok(Some(r)) = row {
                let x: i32 = r.get("x");
                let y: i32 = r.get("y");
                format!("Read pixel at ({}, {})", x, y)
            } else {
                "Find pixel RGB".to_string()
            }
        }
        "find_bitmap" => {
            "Search for bitmap on screen".to_string()
        }
        "branch" => {
            "BRANCH evaluation".to_string()
        }
        _ => "Unknown step".to_string(),
    }
}

/// POST /automations/{id}
pub async fn post_automation_edit_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<EditAutomationForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let name = form.name.trim();
    if name.is_empty() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let description = form.description.unwrap_or_default().trim().to_string();
    let status = match form.status.as_str() {
        "active" | "archived" | "draft" => form.status,
        _ => "draft".to_string(),
    };

    let res = sqlx::query(
        "UPDATE automations SET name = $1, description = $2, status = $3, updated_at = now() WHERE id = $4",
    )
    .bind(name)
    .bind(&description)
    .bind(&status)
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
            Some(serde_json::json!({ "name": name, "description": description, "status": status })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// GET /automations/{id}/delete
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
pub async fn post_automation_delete_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to("/automations").into_response();
    }

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

    Redirect::to("/automations").into_response()
}

/// GET /automations/{id}/steps/new
pub async fn get_step_type_picker_handler(user: AuthUser, Path(id): Path<i64>) -> impl IntoResponse {
    HtmlTemplate(StepTypePickerTemplate {
        user,
        automation_id: id,
    })
}

/// GET /automations/{id}/steps/new/key_press
pub async fn get_new_key_press_step_handler(
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    HtmlTemplate(StepKeyPressTemplate {
        user,
        csrf_token,
        automation_id: id,
        step_id: None,
        label: String::new(),
        post_delay_seconds: 0.0,
        key_combo: String::new(),
        error: None,
        is_edit: false,
    })
}

/// GET /automations/{id}/steps/new/mouse_click
pub async fn get_new_mouse_click_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewMouseClickQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let variables = fetch_automation_variables(&state.db, id).await;

    HtmlTemplate(StepMouseClickTemplate {
        user,
        csrf_token,
        automation_id: id,
        step_id: None,
        label: String::new(),
        post_delay_seconds: 0.0,
        x_mode: "fixed".to_string(),
        x: query.x,
        x_variable_id: None,
        y_mode: "fixed".to_string(),
        y: query.y,
        y_variable_id: None,
        button: "left".to_string(),
        click_type: "single".to_string(),
        variables,
        error: None,
        is_edit: false,
    })
}

/// POST /automations/{id}/steps
pub async fn post_create_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<MouseClickStepForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let is_mouse_click = form.step_type.as_deref() == Some("mouse_click")
        || (form.x.is_some()
            || form.x_variable_id.is_some()
            || form.y.is_some()
            || form.y_variable_id.is_some()
            || form.x_mode.is_some());

    if is_mouse_click {
        let x_mode = form.x_mode.unwrap_or_else(|| "fixed".to_string());
        let y_mode = form.y_mode.unwrap_or_else(|| "fixed".to_string());

        let (final_x, final_x_var) = match x_mode.as_str() {
            "variable" => (None, form.x_variable_id),
            _ => (form.x, None),
        };

        let (final_y, final_y_var) = match y_mode.as_str() {
            "variable" => (None, form.y_variable_id),
            _ => (form.y, None),
        };

        let button = match form.button.as_deref() {
            Some("right") => "right",
            Some("middle") => "middle",
            _ => "left",
        }
        .to_string();

        let click_type = match form.click_type.as_deref() {
            Some("double") => "double",
            _ => "single",
        }
        .to_string();

        let mut err = None;
        if final_x.is_none() && final_x_var.is_none() {
            err = Some("Please provide a fixed X coordinate or select an X variable.".to_string());
        } else if final_y.is_none() && final_y_var.is_none() {
            err = Some("Please provide a fixed Y coordinate or select a Y variable.".to_string());
        }

        if let Some(error_msg) = err {
            let variables = fetch_automation_variables(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepMouseClickTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x_mode,
                    x: form.x,
                    x_variable_id: form.x_variable_id,
                    y_mode,
                    y: form.y,
                    y_variable_id: form.y_variable_id,
                    button,
                    click_type,
                    variables,
                    error: Some(error_msg),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form.label.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

        let max_pos_row = sqlx::query("SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await;

        let next_pos: f64 = match max_pos_row {
            Ok(r) => {
                let max_pos: Option<f64> = r.get("max_pos");
                max_pos.map(|p| p + 10.0).unwrap_or(10.0)
            }
            _ => 10.0,
        };

        let mut tx = match state.db.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!("Failed to start transaction: {}", e);
                return Redirect::to(&format!("/automations/{}", id)).into_response();
            }
        };

        let step_row = sqlx::query(
            "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'mouse_click', $3, $4) RETURNING id",
        )
        .bind(id)
        .bind(next_pos)
        .bind(&label)
        .bind(post_delay_ms)
        .fetch_one(&mut *tx)
        .await;

        let step_id: i64 = match step_row {
            Ok(r) => r.get("id"),
            Err(e) => {
                tracing::error!("Failed to insert step: {}", e);
                return Redirect::to(&format!("/automations/{}", id)).into_response();
            }
        };

        let detail_res = sqlx::query(
            "INSERT INTO step_mouse_clicks (step_id, x, y, x_variable_id, y_variable_id, button, click_type) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(step_id)
        .bind(final_x)
        .bind(final_y)
        .bind(final_x_var)
        .bind(final_y_var)
        .bind(&button)
        .bind(&click_type)
        .execute(&mut *tx)
        .await;

        if let Err(e) = detail_res {
            tracing::error!("Failed to insert step_mouse_clicks: {}", e);
            return Redirect::to(&format!("/automations/{}", id)).into_response();
        }

        let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await;

        if tx.commit().await.is_ok() {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "create_step",
                "automation_step",
                Some(step_id),
                Some(serde_json::json!({
                    "automation_id": id,
                    "step_type": "mouse_click",
                    "x": final_x,
                    "y": final_y,
                    "x_variable_id": final_x_var,
                    "y_variable_id": final_y_var,
                    "button": button,
                    "click_type": click_type
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let key_combo = form.key_combo.as_deref().unwrap_or("").trim();
    if key_combo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(StepKeyPressTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: None,
                label: form.label.unwrap_or_default(),
                post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                key_combo: String::new(),
                error: Some("Key combination cannot be empty.".to_string()),
                is_edit: false,
            }),
        )
            .into_response();
    }

    let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
    let label = form.label.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    let max_pos_row = sqlx::query("SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await;

    let next_pos: f64 = match max_pos_row {
        Ok(r) => {
            let max_pos: Option<f64> = r.get("max_pos");
            max_pos.map(|p| p + 10.0).unwrap_or(10.0)
        }
        _ => 10.0,
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}", id)).into_response();
        }
    };

    let step_row = sqlx::query(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'key_press', $3, $4) RETURNING id",
    )
    .bind(id)
    .bind(next_pos)
    .bind(&label)
    .bind(post_delay_ms)
    .fetch_one(&mut *tx)
    .await;

    let step_id: i64 = match step_row {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to insert step: {}", e);
            return Redirect::to(&format!("/automations/{}", id)).into_response();
        }
    };

    let detail_res = sqlx::query("INSERT INTO step_key_presses (step_id, key_combo) VALUES ($1, $2)")
        .bind(step_id)
        .bind(key_combo)
        .execute(&mut *tx)
        .await;

    if let Err(e) = detail_res {
        tracing::error!("Failed to insert step_key_presses: {}", e);
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let update_auto = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await;

    if let Err(e) = update_auto {
        tracing::error!("Failed to update automation updated_at: {}", e);
    }

    if tx.commit().await.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "create_step",
            "automation_step",
            Some(step_id),
            Some(serde_json::json!({ "automation_id": id, "step_type": "key_press", "key_combo": key_combo })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// GET /automations/{id}/steps/{sid}/edit
pub async fn get_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let step_row = match sqlx::query(
        "SELECT step_type, label, post_delay_ms FROM automation_steps WHERE id = $1 AND automation_id = $2",
    )
    .bind(sid)
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        _ => return Redirect::to(&format!("/automations/{}", id)).into_response(),
    };

    let step_type: String = step_row.get("step_type");
    let label: Option<String> = step_row.get("label");
    let post_delay_ms: i32 = step_row.get("post_delay_ms");

    if step_type == "key_press" {
        let kp_row = sqlx::query("SELECT key_combo FROM step_key_presses WHERE step_id = $1")
            .bind(sid)
            .fetch_optional(&state.db)
            .await;

        let key_combo = match kp_row {
            Ok(Some(r)) => r.get("key_combo"),
            _ => String::new(),
        };

        HtmlTemplate(StepKeyPressTemplate {
            user,
            csrf_token,
            automation_id: id,
            step_id: Some(sid),
            label: label.unwrap_or_default(),
            post_delay_seconds: post_delay_ms as f64 / 1000.0,
            key_combo,
            error: None,
            is_edit: true,
        })
        .into_response()
    } else if step_type == "mouse_click" {
        let mc_row = sqlx::query("SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
            .bind(sid)
            .fetch_optional(&state.db)
            .await;

        let (x, y, x_var_id, y_var_id, button, click_type) = match mc_row {
            Ok(Some(r)) => (
                r.get::<Option<i32>, _>("x"),
                r.get::<Option<i32>, _>("y"),
                r.get::<Option<i64>, _>("x_variable_id"),
                r.get::<Option<i64>, _>("y_variable_id"),
                r.get::<String, _>("button"),
                r.get::<String, _>("click_type"),
            ),
            _ => (None, None, None, None, "left".to_string(), "single".to_string()),
        };

        let x_mode = if x_var_id.is_some() { "variable" } else { "fixed" }.to_string();
        let y_mode = if y_var_id.is_some() { "variable" } else { "fixed" }.to_string();

        let variables = fetch_automation_variables(&state.db, id).await;

        HtmlTemplate(StepMouseClickTemplate {
            user,
            csrf_token,
            automation_id: id,
            step_id: Some(sid),
            label: label.unwrap_or_default(),
            post_delay_seconds: post_delay_ms as f64 / 1000.0,
            x_mode,
            x,
            x_variable_id: x_var_id,
            y_mode,
            y,
            y_variable_id: y_var_id,
            button,
            click_type,
            variables,
            error: None,
            is_edit: true,
        })
        .into_response()
    } else {
        Redirect::to(&format!("/automations/{}", id)).into_response()
    }
}

/// POST /automations/{id}/steps/{sid}
pub async fn post_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
    Form(form): Form<MouseClickStepForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type_row = sqlx::query("SELECT step_type FROM automation_steps WHERE id = $1 AND automation_id = $2")
        .bind(sid)
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    let step_type: String = match step_type_row {
        Ok(Some(r)) => r.get("step_type"),
        _ => return Redirect::to(&format!("/automations/{}", id)).into_response(),
    };

    if step_type == "mouse_click" {
        let x_mode = form.x_mode.unwrap_or_else(|| "fixed".to_string());
        let y_mode = form.y_mode.unwrap_or_else(|| "fixed".to_string());

        let (final_x, final_x_var) = match x_mode.as_str() {
            "variable" => (None, form.x_variable_id),
            _ => (form.x, None),
        };

        let (final_y, final_y_var) = match y_mode.as_str() {
            "variable" => (None, form.y_variable_id),
            _ => (form.y, None),
        };

        let button = match form.button.as_deref() {
            Some("right") => "right",
            Some("middle") => "middle",
            _ => "left",
        }
        .to_string();

        let click_type = match form.click_type.as_deref() {
            Some("double") => "double",
            _ => "single",
        }
        .to_string();

        let mut err = None;
        if final_x.is_none() && final_x_var.is_none() {
            err = Some("Please provide a fixed X coordinate or select an X variable.".to_string());
        } else if final_y.is_none() && final_y_var.is_none() {
            err = Some("Please provide a fixed Y coordinate or select a Y variable.".to_string());
        }

        if let Some(error_msg) = err {
            let variables = fetch_automation_variables(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepMouseClickTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x_mode,
                    x: form.x,
                    x_variable_id: form.x_variable_id,
                    y_mode,
                    y: form.y,
                    y_variable_id: form.y_variable_id,
                    button,
                    click_type,
                    variables,
                    error: Some(error_msg),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form.label.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

        let mut tx = match state.db.begin().await {
            Ok(tx) => tx,
            Err(_) => return Redirect::to(&format!("/automations/{}", id)).into_response(),
        };

        let _ = sqlx::query(
            "UPDATE automation_steps SET label = $1, post_delay_ms = $2, updated_at = now() WHERE id = $3 AND automation_id = $4",
        )
        .bind(&label)
        .bind(post_delay_ms)
        .bind(sid)
        .bind(id)
        .execute(&mut *tx)
        .await;

        let _ = sqlx::query(
            "UPDATE step_mouse_clicks SET x = $1, y = $2, x_variable_id = $3, y_variable_id = $4, button = $5, click_type = $6 WHERE step_id = $7",
        )
        .bind(final_x)
        .bind(final_y)
        .bind(final_x_var)
        .bind(final_y_var)
        .bind(&button)
        .bind(&click_type)
        .bind(sid)
        .execute(&mut *tx)
        .await;

        let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await;

        if tx.commit().await.is_ok() {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "update_step",
                "automation_step",
                Some(sid),
                Some(serde_json::json!({
                    "automation_id": id,
                    "step_type": "mouse_click",
                    "x": final_x,
                    "y": final_y,
                    "x_variable_id": final_x_var,
                    "y_variable_id": final_y_var,
                    "button": button,
                    "click_type": click_type
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let key_combo = form.key_combo.as_deref().unwrap_or("").trim();
    if key_combo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(StepKeyPressTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label: form.label.unwrap_or_default(),
                post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                key_combo: String::new(),
                error: Some("Key combination cannot be empty.".to_string()),
                is_edit: true,
            }),
        )
            .into_response();
    }

    let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
    let label = form.label.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return Redirect::to(&format!("/automations/{}", id)).into_response(),
    };

    let _ = sqlx::query(
        "UPDATE automation_steps SET label = $1, post_delay_ms = $2, updated_at = now() WHERE id = $3 AND automation_id = $4",
    )
    .bind(&label)
    .bind(post_delay_ms)
    .bind(sid)
    .bind(id)
    .execute(&mut *tx)
    .await;

    let _ = sqlx::query("UPDATE step_key_presses SET key_combo = $1 WHERE step_id = $2")
        .bind(key_combo)
        .bind(sid)
        .execute(&mut *tx)
        .await;

    let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await;

    if tx.commit().await.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "update_step",
            "automation_step",
            Some(sid),
            Some(serde_json::json!({ "automation_id": id, "key_combo": key_combo })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// POST /automations/{id}/steps/{sid}/move-up
pub async fn post_move_step_up_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if user.role.can_edit() {
        reorder_step(&state.db, id, sid, true).await;
    }
    Redirect::to(&format!("/automations/{}", id))
}

/// POST /automations/{id}/steps/{sid}/move-down
pub async fn post_move_step_down_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if user.role.can_edit() {
        reorder_step(&state.db, id, sid, false).await;
    }
    Redirect::to(&format!("/automations/{}", id))
}

/// Helper function to reorder step up or down
async fn reorder_step(db: &PgPool, automation_id: i64, step_id: i64, is_up: bool) {
    let steps = match sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(db)
        .await
    {
        Ok(s) => s,
        Err(_) => return,
    };

    let current_idx = steps.iter().position(|r| r.get::<i64, _>("id") == step_id);
    let idx = match current_idx {
        Some(i) => i,
        None => return,
    };

    if is_up && idx == 0 {
        return; // Already at top
    }
    if !is_up && idx == steps.len() - 1 {
        return; // Already at bottom
    }

    let target_idx = if is_up { idx - 1 } else { idx + 1 };

    let pos_curr: f64 = steps[idx].get("position");
    let pos_target: f64 = steps[target_idx].get("position");
    let target_id: i64 = steps[target_idx].get("id");

    // Swap positions
    let mut tx = match db.begin().await {
        Ok(t) => t,
        Err(_) => return,
    };

    let _ = sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
        .bind(pos_target)
        .bind(step_id)
        .execute(&mut *tx)
        .await;

    let _ = sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
        .bind(pos_curr)
        .bind(target_id)
        .execute(&mut *tx)
        .await;

    let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(automation_id)
        .execute(&mut *tx)
        .await;

    let _ = tx.commit().await;

    // Check gap precision & compact if gaps < 0.0001
    check_and_compact_positions(db, automation_id).await;
}

/// POST /automations/{id}/steps/{sid}/delete
pub async fn post_delete_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let res = sqlx::query("DELETE FROM automation_steps WHERE id = $1 AND automation_id = $2")
        .bind(sid)
        .bind(id)
        .execute(&state.db)
        .await;

    if res.is_ok() {
        let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await;

        let _ = log_audit(
            &state.db,
            Some(user.id),
            "delete_step",
            "automation_step",
            Some(sid),
            Some(serde_json::json!({ "automation_id": id })),
        )
        .await;

        check_and_compact_positions(&state.db, id).await;
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// Compact positions to 10.0, 20.0, 30.0... if gaps between adjacent steps are too narrow (< 0.0001)
pub async fn check_and_compact_positions(db: &PgPool, automation_id: i64) {
    let steps = match sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(db)
        .await
    {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut needs_compaction = false;
    for i in 0..steps.len().saturating_sub(1) {
        let pos1: f64 = steps[i].get("position");
        let pos2: f64 = steps[i + 1].get("position");
        if (pos2 - pos1).abs() < 0.0001 {
            needs_compaction = true;
            break;
        }
    }

    if needs_compaction {
        compact_positions(db, automation_id).await;
    }
}

pub async fn compact_positions(db: &PgPool, automation_id: i64) {
    let steps = match sqlx::query("SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(db)
        .await
    {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut tx = match db.begin().await {
        Ok(t) => t,
        Err(_) => return,
    };

    for (i, row) in steps.iter().enumerate() {
        let step_id: i64 = row.get("id");
        let new_pos = (i as f64 + 1.0) * 10.0;
        let _ = sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
            .bind(new_pos)
            .bind(step_id)
            .execute(&mut *tx)
            .await;
    }

    let _ = tx.commit().await;
}
