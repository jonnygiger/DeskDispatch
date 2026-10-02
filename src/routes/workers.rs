use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{FromRow, Row};

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, RequireAdmin};
use crate::AppState;

#[derive(Debug, Clone)]
pub struct WorkerPcItem {
    pub id: i64,
    pub hostname: String,
    pub display_name: String,
    pub status: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub groups: Vec<String>,
}

impl WorkerPcItem {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "online" => "badge-success",
            "busy" => "badge-warning",
            "error" => "badge-danger",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_last_heartbeat(&self) -> String {
        match self.last_heartbeat_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => "Never".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerGroupItem {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub member_count: i64,
    pub member_names: Vec<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorkerGroupSimple {
    pub id: i64,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorkerSimple {
    pub id: i64,
    pub hostname: String,
    pub display_name: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorkerDetailRow {
    pub id: i64,
    pub hostname: String,
    pub display_name: String,
    pub status: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub registration_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkerDetail {
    pub id: i64,
    pub hostname: String,
    pub display_name: String,
    pub status: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub groups: Vec<WorkerGroupSimple>,
    pub registration_token: Option<String>,
}

impl WorkerDetail {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "online" => "badge-success",
            "busy" => "badge-warning",
            "error" => "badge-danger",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_last_heartbeat(&self) -> String {
        match self.last_heartbeat_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => "Never".to_string(),
        }
    }

    pub fn formatted_created_at(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }
}

// Templates

#[derive(Template)]
#[template(path = "workers/index.html")]
pub struct WorkersIndexTemplate {
    pub user: crate::auth::AuthUser,
    pub workers: Vec<WorkerPcItem>,
    pub worker_groups: Vec<WorkerGroupItem>,
}

#[derive(Template)]
#[template(path = "workers/detail.html")]
pub struct WorkerDetailTemplate {
    pub user: crate::auth::AuthUser,
    pub worker: WorkerDetail,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "workers/edit.html")]
pub struct WorkerEditTemplate {
    pub user: crate::auth::AuthUser,
    pub worker: WorkerDetail,
    pub all_groups: Vec<WorkerGroupSimple>,
    pub error: Option<String>,
}

impl WorkerEditTemplate {
    pub fn is_group_selected(&self, group_id: &i64) -> bool {
        self.worker.groups.iter().any(|g| g.id == *group_id)
    }
}

#[derive(Template)]
#[template(path = "workers/group_form.html")]
pub struct WorkerGroupFormTemplate {
    pub user: crate::auth::AuthUser,
    pub group_id: Option<i64>,
    pub name: String,
    pub description: String,
    pub member_worker_ids: Vec<i64>,
    pub all_workers: Vec<WorkerSimple>,
    pub error: Option<String>,
    pub is_edit: bool,
}

impl WorkerGroupFormTemplate {
    pub fn is_worker_selected(&self, worker_id: &i64) -> bool {
        self.member_worker_ids.contains(worker_id)
    }
}

// Form payloads

#[derive(Deserialize)]
pub struct WorkerEditForm {
    pub hostname: String,
    pub display_name: String,
    #[serde(default)]
    pub group_ids: Vec<i64>,
}

#[derive(Deserialize)]
pub struct WorkerGroupForm {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub worker_ids: Vec<i64>,
}

// Handlers

pub async fn get_workers_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let workers_rows = match sqlx::query_as::<_, WorkerDetailRow>(
        r#"
        SELECT
            w.id,
            w.hostname,
            w.display_name,
            w.status,
            w.last_heartbeat_at,
            w.screen_width,
            w.screen_height,
            w.os_info,
            w.agent_version,
            w.created_at,
            w.registration_token
        FROM task_worker_pcs w
        ORDER BY w.display_name ASC, w.id ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Failed to fetch task_worker_pcs: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load workers",
            )
                .into_response();
        }
    };

    let mut workers = Vec::new();
    for row in workers_rows {
        let groups_rows: Vec<String> = sqlx::query_scalar::<_, String>(
            r#"
            SELECT g.name
            FROM worker_groups g
            JOIN worker_group_members m ON g.id = m.group_id
            WHERE m.worker_id = $1
            ORDER BY g.name ASC
            "#,
        )
        .bind(row.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        workers.push(WorkerPcItem {
            id: row.id,
            hostname: row.hostname,
            display_name: row.display_name,
            status: row.status,
            last_heartbeat_at: row.last_heartbeat_at,
            screen_width: row.screen_width,
            screen_height: row.screen_height,
            os_info: row.os_info,
            agent_version: row.agent_version,
            created_at: row.created_at,
            groups: groups_rows,
        });
    }

    let groups_rows = match sqlx::query_as::<_, WorkerGroupSimple>(
        r#"
        SELECT id, name, description
        FROM worker_groups
        ORDER BY name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Failed to fetch worker_groups: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load worker groups",
            )
                .into_response();
        }
    };

    let mut worker_groups = Vec::new();
    for row in groups_rows {
        let members: Vec<String> = sqlx::query_scalar::<_, String>(
            r#"
            SELECT w.display_name
            FROM task_worker_pcs w
            JOIN worker_group_members m ON w.id = m.worker_id
            WHERE m.group_id = $1
            ORDER BY w.display_name ASC
            "#,
        )
        .bind(row.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let member_count = members.len() as i64;

        worker_groups.push(WorkerGroupItem {
            id: row.id,
            name: row.name,
            description: row.description,
            member_count,
            member_names: members,
        });
    }

    HtmlTemplate(WorkersIndexTemplate {
        user,
        workers,
        worker_groups,
    })
    .into_response()
}

pub async fn get_worker_detail_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let row = match sqlx::query_as::<_, WorkerDetailRow>(
        r#"
        SELECT
            id, hostname, display_name, status, last_heartbeat_at,
            screen_width, screen_height, os_info, agent_version,
            created_at, registration_token
        FROM task_worker_pcs
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Worker PC not found").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to fetch worker PC detail: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let groups: Vec<WorkerGroupSimple> = sqlx::query_as::<_, WorkerGroupSimple>(
        r#"
        SELECT g.id, g.name, g.description
        FROM worker_groups g
        JOIN worker_group_members m ON g.id = m.group_id
        WHERE m.worker_id = $1
        ORDER BY g.name ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let worker = WorkerDetail {
        id: row.id,
        hostname: row.hostname,
        display_name: row.display_name,
        status: row.status,
        last_heartbeat_at: row.last_heartbeat_at,
        screen_width: row.screen_width,
        screen_height: row.screen_height,
        os_info: row.os_info,
        agent_version: row.agent_version,
        created_at: row.created_at,
        groups,
        registration_token: row.registration_token,
    };

    HtmlTemplate(WorkerDetailTemplate {
        user,
        worker,
        error: None,
    })
    .into_response()
}

pub async fn get_edit_worker_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let row = match sqlx::query_as::<_, WorkerDetailRow>(
        r#"
        SELECT
            id, hostname, display_name, status, last_heartbeat_at,
            screen_width, screen_height, os_info, agent_version,
            created_at, registration_token
        FROM task_worker_pcs
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Worker PC not found").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to fetch worker PC for edit: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let assigned_groups: Vec<WorkerGroupSimple> = sqlx::query_as::<_, WorkerGroupSimple>(
        r#"
        SELECT g.id, g.name, g.description
        FROM worker_groups g
        JOIN worker_group_members m ON g.id = m.group_id
        WHERE m.worker_id = $1
        ORDER BY g.name ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let all_groups: Vec<WorkerGroupSimple> = sqlx::query_as::<_, WorkerGroupSimple>(
        r#"
        SELECT id, name, description
        FROM worker_groups
        ORDER BY name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let worker = WorkerDetail {
        id: row.id,
        hostname: row.hostname,
        display_name: row.display_name,
        status: row.status,
        last_heartbeat_at: row.last_heartbeat_at,
        screen_width: row.screen_width,
        screen_height: row.screen_height,
        os_info: row.os_info,
        agent_version: row.agent_version,
        created_at: row.created_at,
        groups: assigned_groups,
        registration_token: row.registration_token,
    };

    HtmlTemplate(WorkerEditTemplate {
        user,
        worker,
        all_groups,
        error: None,
    })
    .into_response()
}

pub async fn post_edit_worker_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
    Form(form): Form<WorkerEditForm>,
) -> impl IntoResponse {
    let hostname = form.hostname.trim();
    let display_name = form.display_name.trim();

    if hostname.is_empty() || display_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "Hostname and Display Name cannot be empty",
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to begin transaction: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let update_res = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET hostname = $1, display_name = $2
        WHERE id = $3
        "#,
    )
    .bind(hostname)
    .bind(display_name)
    .bind(id)
    .execute(&mut *tx)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update worker PC: {}", e);
        let _ = tx.rollback().await;
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to update worker PC").into_response();
    }

    // Replace group memberships
    if let Err(e) = sqlx::query("DELETE FROM worker_group_members WHERE worker_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
    {
        tracing::error!("Failed to clear existing worker groups: {}", e);
        let _ = tx.rollback().await;
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    for group_id in &form.group_ids {
        if let Err(e) = sqlx::query(
            "INSERT INTO worker_group_members (worker_id, group_id) VALUES ($1, $2)",
        )
        .bind(id)
        .bind(group_id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!("Failed to assign worker group {}: {}", group_id, e);
            let _ = tx.rollback().await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit transaction: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "update",
        "task_worker_pc",
        Some(id),
        Some(serde_json::json!({
            "hostname": hostname,
            "display_name": display_name,
            "group_ids": form.group_ids,
        })),
    )
    .await;

    Redirect::to(&format!("/workers/{}", id)).into_response()
}

pub async fn post_delete_worker_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "delete",
                "task_worker_pc",
                Some(id),
                None,
            )
            .await;
            Redirect::to("/workers").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete worker PC: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete worker PC").into_response()
        }
    }
}

pub async fn get_new_worker_group_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let all_workers: Vec<WorkerSimple> = sqlx::query_as::<_, WorkerSimple>(
        r#"
        SELECT id, hostname, display_name
        FROM task_worker_pcs
        ORDER BY display_name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    HtmlTemplate(WorkerGroupFormTemplate {
        user,
        group_id: None,
        name: String::new(),
        description: String::new(),
        member_worker_ids: Vec::new(),
        all_workers,
        error: None,
        is_edit: false,
    })
    .into_response()
}

pub async fn post_create_worker_group_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Form(form): Form<WorkerGroupForm>,
) -> impl IntoResponse {
    let name = form.name.trim();
    let description = form.description.trim();

    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "Group name cannot be empty",
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to begin transaction: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let group_row = sqlx::query(
        r#"
        INSERT INTO worker_groups (name, description)
        VALUES ($1, $2)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(description)
    .fetch_one(&mut *tx)
    .await;

    let group_id: i64 = match group_row {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to create worker group: {}", e);
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create worker group (name may already exist)",
            )
                .into_response();
        }
    };

    for worker_id in &form.worker_ids {
        if let Err(e) = sqlx::query(
            "INSERT INTO worker_group_members (worker_id, group_id) VALUES ($1, $2)",
        )
        .bind(worker_id)
        .bind(group_id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!("Failed to insert worker group member: {}", e);
            let _ = tx.rollback().await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit transaction: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create",
        "worker_group",
        Some(group_id),
        Some(serde_json::json!({
            "name": name,
            "description": description,
            "worker_ids": form.worker_ids,
        })),
    )
    .await;

    Redirect::to("/workers").into_response()
}

pub async fn get_edit_worker_group_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let group_row = match sqlx::query_as::<_, WorkerGroupSimple>(
        r#"
        SELECT id, name, description
        FROM worker_groups
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Worker group not found").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to fetch worker group: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let member_worker_ids: Vec<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT worker_id FROM worker_group_members WHERE group_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let all_workers: Vec<WorkerSimple> = sqlx::query_as::<_, WorkerSimple>(
        r#"
        SELECT id, hostname, display_name
        FROM task_worker_pcs
        ORDER BY display_name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    HtmlTemplate(WorkerGroupFormTemplate {
        user,
        group_id: Some(group_row.id),
        name: group_row.name,
        description: group_row.description,
        member_worker_ids,
        all_workers,
        error: None,
        is_edit: true,
    })
    .into_response()
}

pub async fn post_edit_worker_group_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
    Form(form): Form<WorkerGroupForm>,
) -> impl IntoResponse {
    let name = form.name.trim();
    let description = form.description.trim();

    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "Group name cannot be empty",
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to begin transaction: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let update_res = sqlx::query(
        r#"
        UPDATE worker_groups
        SET name = $1, description = $2
        WHERE id = $3
        "#,
    )
    .bind(name)
    .bind(description)
    .bind(id)
    .execute(&mut *tx)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update worker group: {}", e);
        let _ = tx.rollback().await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to update worker group (name may already exist)",
        )
            .into_response();
    }

    if let Err(e) = sqlx::query("DELETE FROM worker_group_members WHERE group_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
    {
        tracing::error!("Failed to clear existing worker group members: {}", e);
        let _ = tx.rollback().await;
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    for worker_id in &form.worker_ids {
        if let Err(e) = sqlx::query(
            "INSERT INTO worker_group_members (worker_id, group_id) VALUES ($1, $2)",
        )
        .bind(worker_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!("Failed to insert worker group member: {}", e);
            let _ = tx.rollback().await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit transaction: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "update",
        "worker_group",
        Some(id),
        Some(serde_json::json!({
            "name": name,
            "description": description,
            "worker_ids": form.worker_ids,
        })),
    )
    .await;

    Redirect::to("/workers").into_response()
}

pub async fn post_delete_worker_group_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM worker_groups WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "delete",
                "worker_group",
                Some(id),
                None,
            )
            .await;
            Redirect::to("/workers").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete worker group: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete worker group").into_response()
        }
    }
}
