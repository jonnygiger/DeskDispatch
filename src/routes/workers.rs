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
    pub fn derived_status(&self) -> &str {
        if self.status == "offline" {
            return "offline";
        }
        match self.last_heartbeat_at {
            Some(dt) => {
                let seconds = Utc::now().signed_duration_since(dt).num_seconds();
                if seconds > 90 {
                    "offline"
                } else {
                    self.status.as_str()
                }
            }
            None => "offline",
        }
    }

    pub fn status_badge_class(&self) -> &'static str {
        match self.derived_status() {
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

#[tracing::instrument(skip(state, user))]
pub async fn post_deactivate_worker_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let empty_api_key_hash: Vec<u8> = Vec::new();
    let result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET status = 'offline', api_key_hash = $1, registration_token = NULL
        WHERE id = $2
        "#,
    )
    .bind(&empty_api_key_hash)
    .bind(id)
    .execute(&state.db)
    .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                return (StatusCode::NOT_FOUND, "Worker PC not found").into_response();
            }
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "deactivate",
                "task_worker_pc",
                Some(id),
                None,
            )
            .await;
            Redirect::to(&format!("/workers/{}", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to deactivate worker PC: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to deactivate worker PC").into_response()
        }
    }
}

#[tracing::instrument(skip(state, user))]
pub async fn post_rotate_worker_key_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let new_token = uuid::Uuid::new_v4().to_string();
    let empty_api_key_hash: Vec<u8> = Vec::new();

    let result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET status = 'offline', api_key_hash = $1, registration_token = $2
        WHERE id = $3
        "#,
    )
    .bind(&empty_api_key_hash)
    .bind(&new_token)
    .bind(id)
    .execute(&state.db)
    .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                return (StatusCode::NOT_FOUND, "Worker PC not found").into_response();
            }
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "rotate_key",
                "task_worker_pc",
                Some(id),
                Some(serde_json::json!({
                    "registration_token": new_token,
                })),
            )
            .await;
            Redirect::to(&format!("/workers/{}", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to rotate worker key: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to rotate worker key").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthUser;

    #[test]
    fn test_worker_new_template_rendering() {
        let user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: crate::auth::UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf_token".to_string(),
        };

        let tmpl = WorkerNewTemplate {
            user,
            hostname: "worker-01.local".to_string(),
            display_name: "Worker 1".to_string(),
            error: Some("Test error message".to_string()),
        };

        let rendered = tmpl.render().expect("Failed to render WorkerNewTemplate");
        assert!(rendered.contains("Register New Task Worker PC"));
        assert!(rendered.contains("worker-01.local"));
        assert!(rendered.contains("Worker 1"));
        assert!(rendered.contains("Test error message"));
    }

    #[test]
    fn test_worker_detail_template_rendering_with_admin_actions() {
        let user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: crate::auth::UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf_token".to_string(),
        };

        let worker = WorkerDetail {
            id: 42,
            hostname: "worker-42.local".to_string(),
            display_name: "Worker 42".to_string(),
            status: "online".to_string(),
            last_heartbeat_at: Some(Utc::now()),
            screen_width: Some(1920),
            screen_height: Some(1080),
            os_info: Some("Windows 11".to_string()),
            agent_version: Some("1.0.0".to_string()),
            created_at: Utc::now(),
            groups: vec![],
            registration_token: Some("sample-token-uuid".to_string()),
        };

        let tmpl = WorkerDetailTemplate {
            user,
            worker,
            error: None,
        };

        let rendered = tmpl.render().expect("Failed to render WorkerDetailTemplate");
        assert!(rendered.contains("Worker 42"));
        assert!(rendered.contains("action=\"/workers/42/rotate-key\""));
        assert!(rendered.contains("action=\"/workers/42/deactivate\""));
        assert!(rendered.contains("Rotate API Key"));
        assert!(rendered.contains("Deactivate"));
    }

    #[test]
    fn test_worker_derived_status() {
        use chrono::Duration;

        let recent_hb = Some(Utc::now());
        let stale_hb = Some(Utc::now() - Duration::seconds(100));

        let worker_online = WorkerPcItem {
            id: 1,
            hostname: "w1".to_string(),
            display_name: "Worker 1".to_string(),
            status: "online".to_string(),
            last_heartbeat_at: recent_hb,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: Utc::now(),
            groups: vec![],
        };
        assert_eq!(worker_online.derived_status(), "online");
        assert_eq!(worker_online.status_badge_class(), "badge-success");

        let worker_stale = WorkerPcItem {
            id: 2,
            hostname: "w2".to_string(),
            display_name: "Worker 2".to_string(),
            status: "online".to_string(),
            last_heartbeat_at: stale_hb,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: Utc::now(),
            groups: vec![],
        };
        assert_eq!(worker_stale.derived_status(), "offline");
        assert_eq!(worker_stale.status_badge_class(), "badge-neutral");

        let worker_no_hb = WorkerPcItem {
            id: 3,
            hostname: "w3".to_string(),
            display_name: "Worker 3".to_string(),
            status: "online".to_string(),
            last_heartbeat_at: None,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: Utc::now(),
            groups: vec![],
        };
        assert_eq!(worker_no_hb.derived_status(), "offline");

        let worker_offline_explicit = WorkerPcItem {
            id: 4,
            hostname: "w4".to_string(),
            display_name: "Worker 4".to_string(),
            status: "offline".to_string(),
            last_heartbeat_at: recent_hb,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: Utc::now(),
            groups: vec![],
        };
        assert_eq!(worker_offline_explicit.derived_status(), "offline");

        let worker_busy = WorkerDetail {
            id: 5,
            hostname: "w5".to_string(),
            display_name: "Worker 5".to_string(),
            status: "busy".to_string(),
            last_heartbeat_at: recent_hb,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: Utc::now(),
            groups: vec![],
            registration_token: None,
        };
        assert_eq!(worker_busy.derived_status(), "busy");
        assert_eq!(worker_busy.status_badge_class(), "badge-warning");
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
    pub fn derived_status(&self) -> &str {
        if self.status == "offline" {
            return "offline";
        }
        match self.last_heartbeat_at {
            Some(dt) => {
                let seconds = Utc::now().signed_duration_since(dt).num_seconds();
                if seconds > 90 {
                    "offline"
                } else {
                    self.status.as_str()
                }
            }
            None => "offline",
        }
    }

    pub fn status_badge_class(&self) -> &'static str {
        match self.derived_status() {
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
#[template(path = "workers/new.html")]
pub struct WorkerNewTemplate {
    pub user: crate::auth::AuthUser,
    pub hostname: String,
    pub display_name: String,
    pub error: Option<String>,
}

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
pub struct WorkerCreateForm {
    pub hostname: String,
    pub display_name: String,
}

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

#[tracing::instrument(skip(user))]
pub async fn get_new_worker_handler(
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    HtmlTemplate(WorkerNewTemplate {
        user,
        hostname: String::new(),
        display_name: String::new(),
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_worker_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Form(form): Form<WorkerCreateForm>,
) -> impl IntoResponse {
    let hostname = form.hostname.trim();
    let display_name = form.display_name.trim();

    if hostname.is_empty() || display_name.is_empty() {
        return HtmlTemplate(WorkerNewTemplate {
            user,
            hostname: hostname.to_string(),
            display_name: display_name.to_string(),
            error: Some("Hostname and Display Name are required.".to_string()),
        })
        .into_response();
    }

    let token = uuid::Uuid::new_v4().to_string();
    let empty_api_key_hash: Vec<u8> = Vec::new();

    let row = sqlx::query(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, api_key_hash, registration_token, status)
        VALUES ($1, $2, $3, $4, 'offline')
        RETURNING id
        "#,
    )
    .bind(hostname)
    .bind(display_name)
    .bind(&empty_api_key_hash)
    .bind(&token)
    .fetch_one(&state.db)
    .await;

    let worker_id: i64 = match row {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to create task_worker_pc: {}", e);
            return HtmlTemplate(WorkerNewTemplate {
                user,
                hostname: hostname.to_string(),
                display_name: display_name.to_string(),
                error: Some("Failed to register worker PC in database.".to_string()),
            })
            .into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create",
        "task_worker_pc",
        Some(worker_id),
        Some(serde_json::json!({
            "hostname": hostname,
            "display_name": display_name,
            "registration_token": token,
        })),
    )
    .await;

    Redirect::to(&format!("/workers/{}", worker_id)).into_response()
}

#[tracing::instrument(skip(state, user))]
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

    // Optimization: Bulk-fetch worker group memberships in a single query to eliminate N+1 DB queries per worker
    let mut worker_groups_map: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    if let Ok(group_mappings) = sqlx::query(
        r#"
        SELECT m.worker_id, g.name
        FROM worker_group_members m
        JOIN worker_groups g ON g.id = m.group_id
        ORDER BY g.name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        for r in group_mappings {
            let worker_id: i64 = r.get("worker_id");
            let group_name: String = r.get("name");
            worker_groups_map.entry(worker_id).or_default().push(group_name);
        }
    }

    let mut workers = Vec::with_capacity(workers_rows.len());
    for row in workers_rows {
        let groups_rows = worker_groups_map.remove(&row.id).unwrap_or_default();

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

    // Optimization: Bulk-fetch group members in a single query to eliminate N+1 DB queries per group
    let mut group_members_map: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    if let Ok(member_mappings) = sqlx::query(
        r#"
        SELECT m.group_id, w.display_name
        FROM worker_group_members m
        JOIN task_worker_pcs w ON w.id = m.worker_id
        ORDER BY w.display_name ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        for r in member_mappings {
            let group_id: i64 = r.get("group_id");
            let display_name: String = r.get("display_name");
            group_members_map.entry(group_id).or_default().push(display_name);
        }
    }

    let mut worker_groups = Vec::with_capacity(groups_rows.len());
    for row in groups_rows {
        let members = group_members_map.remove(&row.id).unwrap_or_default();
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

#[tracing::instrument(skip(state, user))]
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

#[tracing::instrument(skip(state, user))]
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

#[tracing::instrument(skip(state, user, form))]
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

#[tracing::instrument(skip(state, user))]
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

#[tracing::instrument(skip(state, user))]
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

#[tracing::instrument(skip(state, user, form))]
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

#[tracing::instrument(skip(state, user))]
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

#[tracing::instrument(skip(state, user, form))]
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

#[tracing::instrument(skip(state, user))]
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
