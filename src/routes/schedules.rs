use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{DateTime, Utc};
use croner::Cron;
use serde::Deserialize;
use sqlx::{FromRow, Row};
use std::str::FromStr;

use super::auth::HtmlTemplate;
use crate::auth::{AuthUser, CsrfForm, RequireEditor, log_audit};
use axum::Router;
use axum::routing::{get, post};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/schedules",
            get(get_schedules_handler).post(post_create_schedule_handler),
        )
        .route("/schedules/new", get(get_new_schedule_handler))
        .route("/schedules/{id}/edit", get(get_edit_schedule_handler))
        .route("/schedules/{id}", post(post_edit_schedule_handler))
        .route("/schedules/{id}/toggle", post(post_toggle_schedule_handler))
        .route("/schedules/{id}/delete", post(post_delete_schedule_handler))
}

#[derive(Debug, Clone)]
pub struct ScheduleItem {
    pub id: i64,
    pub automation_id: i64,
    pub automation_name: String,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub worker_group_id: Option<i64>,
    pub worker_group_name: Option<String>,
    pub overlap_policy: String,
    pub max_queue_age_secs: Option<i32>,
    pub is_enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_by: i64,
    pub created_at: DateTime<Utc>,
}

impl ScheduleItem {
    pub fn formatted_next_run(&self) -> String {
        match self.next_run_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => {
                if self.is_enabled {
                    "Pending".to_string()
                } else {
                    "Disabled".to_string()
                }
            }
        }
    }

    pub fn formatted_last_run(&self) -> String {
        match self.last_run_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => "Never".to_string(),
        }
    }

    pub fn status_badge_class(&self) -> &'static str {
        if self.is_enabled {
            "badge-success"
        } else {
            "badge-neutral"
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ScheduleAutomationOption {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct ScheduleWorkerGroupOption {
    pub id: i64,
    pub name: String,
}

#[derive(Template)]
#[template(path = "schedules/index.html")]
pub struct SchedulesIndexTemplate {
    pub user: AuthUser,
    pub schedules: Vec<ScheduleItem>,
}

#[derive(Template)]
#[template(path = "schedules/form.html")]
pub struct ScheduleFormTemplate {
    pub user: AuthUser,
    pub is_edit: bool,
    pub schedule_id: Option<i64>,
    pub automation_id: Option<i64>,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub worker_group_id: Option<i64>,
    pub overlap_policy: String,
    pub max_queue_age_secs: Option<i32>,
    pub is_enabled: bool,
    pub automations: Vec<ScheduleAutomationOption>,
    pub worker_groups: Vec<ScheduleWorkerGroupOption>,
    pub error: Option<String>,
}

impl ScheduleFormTemplate {
    pub fn is_automation_selected(&self, id: &i64) -> bool {
        self.automation_id == Some(*id)
    }

    pub fn is_worker_group_selected(&self, id: &i64) -> bool {
        self.worker_group_id == Some(*id)
    }
}

#[derive(Deserialize)]
pub struct ScheduleForm {
    pub automation_id: i64,
    pub name: String,
    pub cron_expression: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    pub worker_group_id: Option<i64>,
    #[serde(default = "default_overlap_policy")]
    pub overlap_policy: String,
    pub max_queue_age_secs: Option<i32>,
    #[serde(default)]
    pub is_enabled: bool,
}

fn default_overlap_policy() -> String {
    "allow".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

pub fn compute_next_run_at(
    cron_expr: &str,
    tz_str: &str,
    from_dt: &DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let tz_name = if tz_str.trim().is_empty() {
        "UTC"
    } else {
        tz_str.trim()
    };

    let tz: chrono_tz::Tz = tz_name
        .parse()
        .map_err(|_| format!("Invalid timezone: '{}'", tz_name))?;

    let cron = Cron::from_str(cron_expr).map_err(|e| format!("Invalid cron expression: {}", e))?;

    let local_from = from_dt.with_timezone(&tz);

    cron.find_next_occurrence(&local_from, false)
        .map_err(|e| format!("Failed to compute next run time: {}", e))
        .map(|dt| dt.with_timezone(&Utc))
}

#[tracing::instrument(skip(state, user))]
pub async fn get_schedules_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    let rows = match sqlx::query(
        r#"
        SELECT
            s.id, s.automation_id, a.name AS automation_name,
            s.name, s.cron_expression, s.timezone,
            s.worker_group_id, wg.name AS worker_group_name,
            s.overlap_policy, s.max_queue_age_secs,
            s.is_enabled, s.next_run_at, s.last_run_at,
            s.created_by, s.created_at
        FROM schedules s
        JOIN automations a ON s.automation_id = a.id
        LEFT JOIN worker_groups wg ON s.worker_group_id = wg.id
        ORDER BY s.id DESC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Failed to fetch schedules: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load schedules",
            )
                .into_response();
        }
    };

    let schedules = rows
        .into_iter()
        .map(|r| ScheduleItem {
            id: r.get("id"),
            automation_id: r.get("automation_id"),
            automation_name: r.get("automation_name"),
            name: r.get("name"),
            cron_expression: r.get("cron_expression"),
            timezone: r.get("timezone"),
            worker_group_id: r.get("worker_group_id"),
            worker_group_name: r.get("worker_group_name"),
            overlap_policy: r.get("overlap_policy"),
            max_queue_age_secs: r.get("max_queue_age_secs"),
            is_enabled: r.get("is_enabled"),
            next_run_at: r.get("next_run_at"),
            last_run_at: r.get("last_run_at"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        })
        .collect();

    HtmlTemplate(SchedulesIndexTemplate { user, schedules }).into_response()
}

/// Helper function to fetch schedule form dropdown options (automations and worker groups)
/// concurrently using `tokio::join!`. This reduces HTTP response latency by avoiding sequential database round-trips.
async fn fetch_schedule_options(
    db: &sqlx::PgPool,
) -> (
    Vec<ScheduleAutomationOption>,
    Vec<ScheduleWorkerGroupOption>,
) {
    let (automations_res, worker_groups_res) = tokio::join!(
        sqlx::query_as::<_, ScheduleAutomationOption>(
            "SELECT id, name FROM automations WHERE status != 'archived' ORDER BY name ASC",
        )
        .fetch_all(db),
        sqlx::query_as::<_, ScheduleWorkerGroupOption>(
            "SELECT id, name FROM worker_groups ORDER BY name ASC",
        )
        .fetch_all(db),
    );

    (
        automations_res.unwrap_or_default(),
        worker_groups_res.unwrap_or_default(),
    )
}

#[tracing::instrument(skip(state, user))]
pub async fn get_new_schedule_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let (automations, worker_groups) = fetch_schedule_options(&state.db).await;

    HtmlTemplate(ScheduleFormTemplate {
        user,
        is_edit: false,
        schedule_id: None,
        automation_id: None,
        name: String::new(),
        cron_expression: String::new(),
        timezone: "UTC".to_string(),
        worker_group_id: None,
        overlap_policy: "allow".to_string(),
        max_queue_age_secs: None,
        is_enabled: true,
        automations,
        worker_groups,
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_schedule_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<ScheduleForm>,
) -> impl IntoResponse {
    let name = form.name.trim();
    let cron_expression = form.cron_expression.trim();
    let timezone = if form.timezone.trim().is_empty() {
        "UTC"
    } else {
        form.timezone.trim()
    };

    let (automations, worker_groups) = fetch_schedule_options(&state.db).await;

    if name.is_empty() || cron_expression.is_empty() {
        return HtmlTemplate(ScheduleFormTemplate {
            user,
            is_edit: false,
            schedule_id: None,
            automation_id: Some(form.automation_id),
            name: name.to_string(),
            cron_expression: cron_expression.to_string(),
            timezone: timezone.to_string(),
            worker_group_id: form.worker_group_id,
            overlap_policy: form.overlap_policy.clone(),
            max_queue_age_secs: form.max_queue_age_secs,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some("Schedule Name and Cron Expression are required.".to_string()),
        })
        .into_response();
    }

    let now = Utc::now();
    let next_run_at = if form.is_enabled {
        match compute_next_run_at(cron_expression, timezone, &now) {
            Ok(next_dt) => Some(next_dt),
            Err(err_msg) => {
                return HtmlTemplate(ScheduleFormTemplate {
                    user,
                    is_edit: false,
                    schedule_id: None,
                    automation_id: Some(form.automation_id),
                    name: name.to_string(),
                    cron_expression: cron_expression.to_string(),
                    timezone: timezone.to_string(),
                    worker_group_id: form.worker_group_id,
                    overlap_policy: form.overlap_policy.clone(),
                    max_queue_age_secs: form.max_queue_age_secs,
                    is_enabled: form.is_enabled,
                    automations,
                    worker_groups,
                    error: Some(err_msg),
                })
                .into_response();
            }
        }
    } else {
        None
    };

    let row_res = sqlx::query(
        r#"
        INSERT INTO schedules (
            automation_id, name, cron_expression, timezone,
            worker_group_id, overlap_policy, max_queue_age_secs,
            is_enabled, next_run_at, created_by
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id
        "#,
    )
    .bind(form.automation_id)
    .bind(name)
    .bind(cron_expression)
    .bind(timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.is_enabled)
    .bind(next_run_at)
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let schedule_id: i64 = match row_res {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to create schedule: {}", e);
            return HtmlTemplate(ScheduleFormTemplate {
                user,
                is_edit: false,
                schedule_id: None,
                automation_id: Some(form.automation_id),
                name: name.to_string(),
                cron_expression: cron_expression.to_string(),
                timezone: timezone.to_string(),
                worker_group_id: form.worker_group_id,
                overlap_policy: form.overlap_policy.clone(),
                max_queue_age_secs: form.max_queue_age_secs,
                is_enabled: form.is_enabled,
                automations,
                worker_groups,
                error: Some("Database error creating schedule.".to_string()),
            })
            .into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create",
        "schedule",
        Some(schedule_id),
        Some(serde_json::json!({
            "automation_id": form.automation_id,
            "name": name,
            "cron_expression": cron_expression,
            "timezone": timezone,
            "worker_group_id": form.worker_group_id,
            "is_enabled": form.is_enabled,
        })),
    )
    .await;

    Redirect::to("/schedules").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn get_edit_schedule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let row = match sqlx::query(
        r#"
        SELECT id, automation_id, name, cron_expression, timezone, worker_group_id,
               overlap_policy, max_queue_age_secs, is_enabled
        FROM schedules
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "Schedule not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch schedule: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let (automations, worker_groups) = fetch_schedule_options(&state.db).await;

    HtmlTemplate(ScheduleFormTemplate {
        user,
        is_edit: true,
        schedule_id: Some(id),
        automation_id: Some(row.get("automation_id")),
        name: row.get("name"),
        cron_expression: row.get("cron_expression"),
        timezone: row.get("timezone"),
        worker_group_id: row.get("worker_group_id"),
        overlap_policy: row.get("overlap_policy"),
        max_queue_age_secs: row.get("max_queue_age_secs"),
        is_enabled: row.get("is_enabled"),
        automations,
        worker_groups,
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_edit_schedule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<ScheduleForm>,
) -> impl IntoResponse {
    let name = form.name.trim();
    let cron_expression = form.cron_expression.trim();
    let timezone = if form.timezone.trim().is_empty() {
        "UTC"
    } else {
        form.timezone.trim()
    };

    let (automations, worker_groups) = fetch_schedule_options(&state.db).await;

    if name.is_empty() || cron_expression.is_empty() {
        return HtmlTemplate(ScheduleFormTemplate {
            user,
            is_edit: true,
            schedule_id: Some(id),
            automation_id: Some(form.automation_id),
            name: name.to_string(),
            cron_expression: cron_expression.to_string(),
            timezone: timezone.to_string(),
            worker_group_id: form.worker_group_id,
            overlap_policy: form.overlap_policy.clone(),
            max_queue_age_secs: form.max_queue_age_secs,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some("Schedule Name and Cron Expression are required.".to_string()),
        })
        .into_response();
    }

    let now = Utc::now();
    let next_run_at = if form.is_enabled {
        match compute_next_run_at(cron_expression, timezone, &now) {
            Ok(next_dt) => Some(next_dt),
            Err(err_msg) => {
                return HtmlTemplate(ScheduleFormTemplate {
                    user,
                    is_edit: true,
                    schedule_id: Some(id),
                    automation_id: Some(form.automation_id),
                    name: name.to_string(),
                    cron_expression: cron_expression.to_string(),
                    timezone: timezone.to_string(),
                    worker_group_id: form.worker_group_id,
                    overlap_policy: form.overlap_policy.clone(),
                    max_queue_age_secs: form.max_queue_age_secs,
                    is_enabled: form.is_enabled,
                    automations,
                    worker_groups,
                    error: Some(err_msg),
                })
                .into_response();
            }
        }
    } else {
        None
    };

    let update_res = sqlx::query(
        r#"
        UPDATE schedules
        SET automation_id = $1,
            name = $2,
            cron_expression = $3,
            timezone = $4,
            worker_group_id = $5,
            overlap_policy = $6,
            max_queue_age_secs = $7,
            is_enabled = $8,
            next_run_at = $9
        WHERE id = $10
        "#,
    )
    .bind(form.automation_id)
    .bind(name)
    .bind(cron_expression)
    .bind(timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.is_enabled)
    .bind(next_run_at)
    .bind(id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update schedule: {}", e);
        return HtmlTemplate(ScheduleFormTemplate {
            user,
            is_edit: true,
            schedule_id: Some(id),
            automation_id: Some(form.automation_id),
            name: name.to_string(),
            cron_expression: cron_expression.to_string(),
            timezone: timezone.to_string(),
            worker_group_id: form.worker_group_id,
            overlap_policy: form.overlap_policy.clone(),
            max_queue_age_secs: form.max_queue_age_secs,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some("Database error updating schedule.".to_string()),
        })
        .into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "update",
        "schedule",
        Some(id),
        Some(serde_json::json!({
            "automation_id": form.automation_id,
            "name": name,
            "cron_expression": cron_expression,
            "timezone": timezone,
            "worker_group_id": form.worker_group_id,
            "is_enabled": form.is_enabled,
        })),
    )
    .await;

    Redirect::to("/schedules").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn post_toggle_schedule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let row = match sqlx::query(
        "SELECT is_enabled, cron_expression, timezone FROM schedules WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "Schedule not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch schedule for toggle: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let current_enabled: bool = row.get("is_enabled");
    let cron_expr: String = row.get("cron_expression");
    let timezone: String = row.get("timezone");
    let new_enabled = !current_enabled;

    let now = Utc::now();
    let next_run_at = if new_enabled {
        compute_next_run_at(&cron_expr, &timezone, &now).ok()
    } else {
        None
    };

    let update_res =
        sqlx::query("UPDATE schedules SET is_enabled = $1, next_run_at = $2 WHERE id = $3")
            .bind(new_enabled)
            .bind(next_run_at)
            .bind(id)
            .execute(&state.db)
            .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to toggle schedule: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "toggle",
        "schedule",
        Some(id),
        Some(serde_json::json!({
            "is_enabled": new_enabled,
        })),
    )
    .await;

    Redirect::to("/schedules").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn post_delete_schedule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM schedules WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            let _ = log_audit(
                &state.db,
                Some(user.id),
                "delete",
                "schedule",
                Some(id),
                None,
            )
            .await;
            Redirect::to("/schedules").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete schedule",
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(pool))]
pub async fn process_due_schedules(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let due_schedules = sqlx::query(
        r#"
        SELECT id, automation_id, cron_expression, timezone, worker_group_id,
               overlap_policy, max_queue_age_secs, next_run_at
        FROM schedules
        WHERE is_enabled = true
          AND next_run_at IS NOT NULL
          AND next_run_at <= now()
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut queued_count = 0u64;

    for sched in due_schedules {
        let schedule_id: i64 = sched.get("id");
        let automation_id: i64 = sched.get("automation_id");
        let cron_expr: String = sched.get("cron_expression");
        let timezone: String = sched.get("timezone");
        let worker_group_id: Option<i64> = sched.get("worker_group_id");
        let overlap_policy: String = sched.get("overlap_policy");
        let max_queue_age_secs: Option<i32> = sched.get("max_queue_age_secs");
        let scheduled_run_at: DateTime<Utc> = sched.get("next_run_at");

        let now = Utc::now();
        let mut should_queue = true;

        // 1. Check max_queue_age_secs expiry
        if let Some(max_age) = max_queue_age_secs {
            let age_secs = (now - scheduled_run_at).num_seconds();
            if age_secs > max_age as i64 {
                tracing::warn!(
                    schedule_id = schedule_id,
                    age_secs = age_secs,
                    max_queue_age_secs = max_age,
                    "Scheduled run expired because it exceeded max_queue_age_secs; skipping execution"
                );
                should_queue = false;
            }
        }

        // 2. Check overlap policy if 'skip'
        if should_queue && overlap_policy == "skip" {
            let active_run_exists: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM task_runs
                    WHERE schedule_id = $1
                      AND status IN ('queued', 'running', 'cancelling')
                )
                "#,
            )
            .bind(schedule_id)
            .fetch_one(&mut *tx)
            .await?;

            if active_run_exists {
                tracing::warn!(
                    schedule_id = schedule_id,
                    "Active task run exists for schedule with overlap_policy='skip'; skipping execution"
                );
                should_queue = false;
            }
        }

        if should_queue {
            sqlx::query(
                r#"
                INSERT INTO task_runs (automation_id, schedule_id, target_worker_group_id, status, queued_at)
                VALUES ($1, $2, $3, 'queued', now())
                "#,
            )
            .bind(automation_id)
            .bind(schedule_id)
            .bind(worker_group_id)
            .execute(&mut *tx)
            .await?;

            queued_count += 1;
        }

        // Recompute next run time loudly on failure
        let next_run = match compute_next_run_at(&cron_expr, &timezone, &now) {
            Ok(next_dt) => Some(next_dt),
            Err(e) => {
                tracing::error!(
                    schedule_id = schedule_id,
                    cron_expression = %cron_expr,
                    timezone = %timezone,
                    error = %e,
                    "FAILED to compute next_run_at for schedule! Schedule next_run_at will be set to NULL."
                );
                None
            }
        };

        if should_queue {
            sqlx::query(
                r#"
                UPDATE schedules
                SET last_run_at = now(),
                    next_run_at = $1
                WHERE id = $2
                "#,
            )
            .bind(next_run)
            .bind(schedule_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE schedules
                SET next_run_at = $1
                WHERE id = $2
                "#,
            )
            .bind(next_run)
            .bind(schedule_id)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;

    if queued_count > 0 {
        tracing::info!(count = queued_count, "Queued scheduled task runs");
    }

    Ok(queued_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_next_run_at_valid_cron() {
        let now = Utc::now();
        let result = compute_next_run_at("0 9 * * MON-FRI", "UTC", &now);
        assert!(
            result.is_ok(),
            "Expected valid cron expression to parse successfully"
        );
        let next_dt = result.unwrap();
        assert!(next_dt > now, "Next run time must be in the future");
    }

    #[test]
    fn test_compute_next_run_at_timezone_support() {
        use chrono::TimeZone;
        // Fix base time: 2025-01-15 08:00:00 UTC
        let base_dt = Utc.with_ymd_and_hms(2025, 1, 15, 8, 0, 0).unwrap();

        // Standard 5-field cron: 0 9 * * * (9:00 AM)
        let res_utc = compute_next_run_at("0 9 * * *", "UTC", &base_dt).unwrap();
        // UTC 9:00 AM on Jan 15, 2025
        assert_eq!(res_utc, Utc.with_ymd_and_hms(2025, 1, 15, 9, 0, 0).unwrap());

        // In America/New_York (EST, UTC-5 in Jan)
        // 9:00 AM EST on Jan 15, 2025 is 14:00:00 UTC
        let res_ny = compute_next_run_at("0 9 * * *", "America/New_York", &base_dt).unwrap();
        assert_eq!(res_ny, Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap());

        // Test DST in America/New_York (EDT, UTC-4 in July)
        let base_july = Utc.with_ymd_and_hms(2025, 7, 15, 8, 0, 0).unwrap();
        // 9:00 AM EDT on July 15, 2025 is 13:00:00 UTC
        let res_july_ny = compute_next_run_at("0 9 * * *", "America/New_York", &base_july).unwrap();
        assert_eq!(
            res_july_ny,
            Utc.with_ymd_and_hms(2025, 7, 15, 13, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_compute_next_run_at_invalid_timezone() {
        let now = Utc::now();
        let result = compute_next_run_at("0 9 * * *", "Invalid/Timezone_Name", &now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid timezone"));
    }

    #[test]
    fn test_compute_next_run_at_invalid_cron() {
        let now = Utc::now();
        let result = compute_next_run_at("invalid cron expression", "UTC", &now);
        assert!(result.is_err(), "Expected invalid cron expression to fail");
    }

    #[test]
    fn test_schedule_item_formatting() {
        use chrono::TimeZone;

        let item = ScheduleItem {
            id: 1,
            automation_id: 10,
            automation_name: "Test Automation".to_string(),
            name: "Daily Run".to_string(),
            cron_expression: "0 0 * * *".to_string(),
            timezone: "UTC".to_string(),
            worker_group_id: None,
            worker_group_name: None,
            overlap_policy: "allow".to_string(),
            max_queue_age_secs: Some(3600),
            is_enabled: true,
            next_run_at: None,
            last_run_at: None,
            created_by: 1,
            created_at: Utc::now(),
        };

        assert_eq!(item.formatted_next_run(), "Pending");
        assert_eq!(item.formatted_last_run(), "Never");
        assert_eq!(item.status_badge_class(), "badge-success");

        let disabled_item = ScheduleItem {
            is_enabled: false,
            ..item.clone()
        };
        assert_eq!(disabled_item.formatted_next_run(), "Disabled");
        assert_eq!(disabled_item.status_badge_class(), "badge-neutral");

        let next_dt = Utc.with_ymd_and_hms(2025, 6, 15, 12, 30, 0).unwrap();
        let last_dt = Utc.with_ymd_and_hms(2025, 6, 15, 10, 15, 0).unwrap();
        let active_item_with_timestamps = ScheduleItem {
            next_run_at: Some(next_dt),
            last_run_at: Some(last_dt),
            ..item
        };
        assert_eq!(
            active_item_with_timestamps.formatted_next_run(),
            "2025-06-15 12:30:00 UTC"
        );
        assert_eq!(
            active_item_with_timestamps.formatted_last_run(),
            "2025-06-15 10:15:00 UTC"
        );
    }

    #[test]
    fn test_schedule_form_template_selection_helpers() {
        let user = AuthUser {
            id: 1,
            username: "editor".to_string(),
            display_name: "Editor User".to_string(),
            role: crate::auth::UserRole::Editor,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "csrf_token_test".to_string(),
            must_change_password: false,
        };

        let template_with_selections = ScheduleFormTemplate {
            user: user.clone(),
            is_edit: true,
            schedule_id: Some(42),
            automation_id: Some(10),
            name: "Nightly Sync".to_string(),
            cron_expression: "0 2 * * *".to_string(),
            timezone: "UTC".to_string(),
            worker_group_id: Some(5),
            overlap_policy: "skip".to_string(),
            max_queue_age_secs: Some(1800),
            is_enabled: true,
            automations: vec![],
            worker_groups: vec![],
            error: None,
        };

        // Assert automation selection helper returns true for matching id and false for non-matching id
        assert!(
            template_with_selections.is_automation_selected(&10),
            "Expected automation_id 10 to be selected"
        );
        assert!(
            !template_with_selections.is_automation_selected(&99),
            "Expected automation_id 99 not to be selected"
        );

        // Assert worker group selection helper returns true for matching id and false for non-matching id
        assert!(
            template_with_selections.is_worker_group_selected(&5),
            "Expected worker_group_id 5 to be selected"
        );
        assert!(
            !template_with_selections.is_worker_group_selected(&99),
            "Expected worker_group_id 99 not to be selected"
        );

        let template_without_selections = ScheduleFormTemplate {
            user,
            is_edit: false,
            schedule_id: None,
            automation_id: None,
            name: String::new(),
            cron_expression: String::new(),
            timezone: "UTC".to_string(),
            worker_group_id: None,
            overlap_policy: "allow".to_string(),
            max_queue_age_secs: None,
            is_enabled: true,
            automations: vec![],
            worker_groups: vec![],
            error: None,
        };

        // Assert selection helpers return false when template selection fields are None
        assert!(
            !template_without_selections.is_automation_selected(&10),
            "Expected false when automation_id is None"
        );
        assert!(
            !template_without_selections.is_worker_group_selected(&5),
            "Expected false when worker_group_id is None"
        );
    }
}
