use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::Row;

use super::auth::HtmlTemplate;
use super::workers::WorkerDetail;
use crate::auth::{log_audit, AuthUser, RequireEditor};
use crate::magnifier::ImageMagnifier;
use crate::AppState;

#[derive(Debug, Clone)]
pub struct RecordingSessionDetail {
    pub id: i64,
    pub worker_id: i64,
    pub worker_display_name: String,
    pub worker_hostname: String,
    pub started_by_user_id: i64,
    pub started_by_display_name: String,
    pub status: String,
    pub resulting_automation_id: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl RecordingSessionDetail {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "recording" => "badge-warning",
            "completed" => "badge-success",
            "imported" => "badge-info",
            "discarded" => "badge-danger",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_started_at(&self) -> String {
        self.started_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    pub fn formatted_ended_at(&self) -> String {
        match self.ended_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => "In progress".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecordingEventDetail {
    pub id: i64,
    pub recording_session_id: i64,
    pub sequence_number: i32,
    pub event_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub button: Option<String>,
    pub key_combo: Option<String>,
    pub screenshot_object_key: Option<String>,
    pub captured_at: DateTime<Utc>,
    pub magnifier: Option<ImageMagnifier>,
}

impl RecordingEventDetail {
    pub fn description(&self) -> String {
        match self.event_type.as_str() {
            "mouse_click" => {
                let btn = self.button.as_deref().unwrap_or("left");
                let px = self.x.unwrap_or(0);
                let py = self.y.unwrap_or(0);
                format!("Mouse Click at ({}, {}) [{}]", px, py, btn)
            }
            "key_press" => {
                let combo = self.key_combo.as_deref().unwrap_or("unknown");
                format!("Key Press: {}", combo)
            }
            "screenshot" => "Screen Capture".to_string(),
            _ => format!("Event: {}", self.event_type),
        }
    }

    pub fn formatted_time(&self) -> String {
        self.captured_at.format("%H:%M:%S.%3f UTC").to_string()
    }
}

// Templates

#[derive(Template)]
#[template(path = "recordings/start.html")]
pub struct RecordingStartTemplate {
    pub user: AuthUser,
    pub worker: WorkerDetail,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "recordings/status.html")]
pub struct RecordingStatusTemplate {
    pub user: AuthUser,
    pub session: RecordingSessionDetail,
    pub event_count: i64,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "recordings/review.html")]
pub struct RecordingReviewTemplate {
    pub user: AuthUser,
    pub session: RecordingSessionDetail,
    pub events: Vec<RecordingEventDetail>,
    pub default_automation_name: String,
    pub error: Option<String>,
}

// Form payloads

#[derive(Deserialize)]
pub struct ConvertRecordingForm {
    pub automation_name: String,
    pub automation_description: Option<String>,
    #[serde(default)]
    pub exclude_event_ids: Vec<i64>,
}

// Handlers

/// GET /workers/{id}/record
#[tracing::instrument(skip(state, user))]
pub async fn get_worker_record_start_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let row = match sqlx::query_as::<_, super::workers::WorkerDetailRow>(
        r#"
        SELECT id, hostname, display_name, status, last_heartbeat_at,
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
        _ => return (StatusCode::NOT_FOUND, "Worker PC not found").into_response(),
    };

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
        groups: Vec::new(),
        registration_token: row.registration_token,
    };

    HtmlTemplate(RecordingStartTemplate {
        user,
        worker,
        error: None,
    })
    .into_response()
}

/// POST /workers/{id}/record/start
#[tracing::instrument(skip(state, user))]
pub async fn post_worker_record_start_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let worker_exists = match sqlx::query("SELECT id FROM task_worker_pcs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(_)) => true,
        _ => false,
    };

    if !worker_exists {
        return (StatusCode::NOT_FOUND, "Worker PC not found").into_response();
    }

    let insert_res = sqlx::query(
        r#"
        INSERT INTO recording_sessions (worker_id, started_by_user_id, status, started_at)
        VALUES ($1, $2, 'recording', now())
        RETURNING id
        "#,
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let session_id: i64 = match insert_res {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to create recording_session: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to start recording session").into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "start_recording_session",
        "recording_session",
        Some(session_id),
        Some(serde_json::json!({ "worker_id": id })),
    )
    .await;

    Redirect::to(&format!("/recordings/{}", session_id)).into_response()
}

/// Helper to load RecordingSessionDetail
#[tracing::instrument(skip(db))]
async fn fetch_recording_session_detail(
    db: &sqlx::PgPool,
    session_id: i64,
) -> Result<Option<RecordingSessionDetail>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            rs.id,
            rs.worker_id,
            w.display_name AS worker_display_name,
            w.hostname AS worker_hostname,
            rs.started_by_user_id,
            u.display_name AS started_by_display_name,
            rs.status,
            rs.resulting_automation_id,
            rs.started_at,
            rs.ended_at
        FROM recording_sessions rs
        JOIN task_worker_pcs w ON rs.worker_id = w.id
        JOIN users u ON rs.started_by_user_id = u.id
        WHERE rs.id = $1
        "#,
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| RecordingSessionDetail {
        id: r.get("id"),
        worker_id: r.get("worker_id"),
        worker_display_name: r.get("worker_display_name"),
        worker_hostname: r.get("worker_hostname"),
        started_by_user_id: r.get("started_by_user_id"),
        started_by_display_name: r.get("started_by_display_name"),
        status: r.get("status"),
        resulting_automation_id: r.get("resulting_automation_id"),
        started_at: r.get("started_at"),
        ended_at: r.get("ended_at"),
    }))
}

/// GET /recordings/{id}
#[tracing::instrument(skip(state, user))]
pub async fn get_recording_status_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: AuthUser,
) -> impl IntoResponse {
    let session = match fetch_recording_session_detail(&state.db, id).await {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/workers").into_response(),
    };

    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recording_events WHERE recording_session_id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    HtmlTemplate(RecordingStatusTemplate {
        user,
        session,
        event_count,
        error: None,
    })
    .into_response()
}

/// POST /recordings/{id}/stop
#[tracing::instrument(skip(state, user))]
pub async fn post_recording_stop_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let update_res = sqlx::query(
        r#"
        UPDATE recording_sessions
        SET status = 'completed',
            ended_at = COALESCE(ended_at, now())
        WHERE id = $1
        RETURNING id
        "#,
    )
    .bind(id)
    .execute(&state.db)
    .await;

    if update_res.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "stop_recording_session",
            "recording_session",
            Some(id),
            None,
        )
        .await;
    }

    Redirect::to(&format!("/recordings/{}/review", id)).into_response()
}

/// GET /recordings/{id}/review
#[tracing::instrument(skip(state, user))]
pub async fn get_recording_review_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let session = match fetch_recording_session_detail(&state.db, id).await {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/workers").into_response(),
    };

    let event_rows = match sqlx::query(
        r#"
        SELECT
            id, recording_session_id, sequence_number, event_type,
            x, y, button, key_combo, screenshot_object_key, captured_at
        FROM recording_events
        WHERE recording_session_id = $1
        ORDER BY sequence_number ASC, id ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching recording_events: {}", e);
            Vec::new()
        }
    };

    let storage = state.storage_service();
    let mut events = Vec::new();

    for r in event_rows {
        let event_id: i64 = r.get("id");
        let recording_session_id: i64 = r.get("recording_session_id");
        let sequence_number: i32 = r.get("sequence_number");
        let event_type: String = r.get("event_type");
        let x: Option<i32> = r.get("x");
        let y: Option<i32> = r.get("y");
        let button: Option<String> = r.get("button");
        let key_combo: Option<String> = r.get("key_combo");
        let screenshot_object_key: Option<String> = r.get("screenshot_object_key");
        let captured_at: DateTime<Utc> = r.get("captured_at");

        let mut magnifier = None;
        if let Some(ref key) = screenshot_object_key {
            if let Ok(presigned_url) = storage.generate_presigned_get_url(key, std::time::Duration::from_secs(3600)).await {
                magnifier = Some(ImageMagnifier::new(
                    presigned_url,
                    1920,
                    1080,
                    x.and_then(|v| u32::try_from(v).ok()),
                    y.and_then(|v| u32::try_from(v).ok()),
                ));
            }
        }

        events.push(RecordingEventDetail {
            id: event_id,
            recording_session_id,
            sequence_number,
            event_type,
            x,
            y,
            button,
            key_combo,
            screenshot_object_key,
            captured_at,
            magnifier,
        });
    }

    let default_automation_name = format!(
        "Recorded Automation - {}",
        session.started_at.format("%Y-%m-%d %H:%M")
    );

    HtmlTemplate(RecordingReviewTemplate {
        user,
        session,
        events,
        default_automation_name,
        error: None,
    })
    .into_response()
}

/// POST /recordings/{id}/discard
#[tracing::instrument(skip(state, user))]
pub async fn post_recording_discard_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
) -> impl IntoResponse {
    let update_res = sqlx::query(
        "UPDATE recording_sessions SET status = 'discarded' WHERE id = $1",
    )
    .bind(id)
    .execute(&state.db)
    .await;

    if update_res.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "discard_recording_session",
            "recording_session",
            Some(id),
            None,
        )
        .await;
    }

    Redirect::to("/automations").into_response()
}

/// POST /recordings/{id}/convert
#[tracing::instrument(skip(state, user, form))]
pub async fn post_recording_convert_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireEditor(user): RequireEditor,
    Form(form): Form<ConvertRecordingForm>,
) -> impl IntoResponse {
    let auto_name = form.automation_name.trim();
    if auto_name.is_empty() {
        return Redirect::to(&format!("/recordings/{}/review", id)).into_response();
    }

    let auto_desc = form.automation_description.unwrap_or_default().trim().to_string();

    let _session = match fetch_recording_session_detail(&state.db, id).await {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/workers").into_response(),
    };

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to begin transaction for recording conversion: {}", e);
            return Redirect::to(&format!("/recordings/{}/review", id)).into_response();
        }
    };

    let new_automation_row = sqlx::query(
        "INSERT INTO automations (name, description, status, created_by) VALUES ($1, $2, 'draft', $3) RETURNING id",
    )
    .bind(auto_name)
    .bind(&auto_desc)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await;

    let new_automation_id: i64 = match new_automation_row {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to create automation during conversion: {}", e);
            let _ = tx.rollback().await;
            return Redirect::to(&format!("/recordings/{}/review", id)).into_response();
        }
    };

    let events = sqlx::query(
        r#"
        SELECT id, sequence_number, event_type, x, y, button, key_combo, screenshot_object_key
        FROM recording_events
        WHERE recording_session_id = $1
          AND id != ALL($2)
        ORDER BY sequence_number ASC, id ASC
        "#,
    )
    .bind(id)
    .bind(&form.exclude_event_ids)
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();

    let mut current_position = 10.0;
    let mut last_created_step_id: Option<i64> = None;

    for event in events {
        let event_type: String = event.get("event_type");
        let x: Option<i32> = event.get("x");
        let y: Option<i32> = event.get("y");
        let button: Option<String> = event.get("button");
        let key_combo: Option<String> = event.get("key_combo");
        let screenshot_object_key: Option<String> = event.get("screenshot_object_key");

        match event_type.as_str() {
            "mouse_click" => {
                let step_row = sqlx::query(
                    "INSERT INTO automation_steps (automation_id, position, step_type, post_delay_ms) VALUES ($1, $2, 'mouse_click', 0) RETURNING id",
                )
                .bind(new_automation_id)
                .bind(current_position)
                .fetch_one(&mut *tx)
                .await;

                if let Ok(r) = step_row {
                    let step_id: i64 = r.get("id");
                    last_created_step_id = Some(step_id);

                    let btn = button.as_deref().unwrap_or("left");
                    let _ = sqlx::query(
                        "INSERT INTO step_mouse_clicks (step_id, x, y, button, click_type) VALUES ($1, $2, $3, $4, 'single')",
                    )
                    .bind(step_id)
                    .bind(x.unwrap_or(0))
                    .bind(y.unwrap_or(0))
                    .bind(btn)
                    .execute(&mut *tx)
                    .await;

                    if let Some(ref key) = screenshot_object_key {
                        let _ = sqlx::query(
                            "INSERT INTO step_screenshots (step_id, object_storage_key, width, height) VALUES ($1, $2, 1920, 1080) ON CONFLICT (step_id) DO UPDATE SET object_storage_key = EXCLUDED.object_storage_key",
                        )
                        .bind(step_id)
                        .bind(key)
                        .execute(&mut *tx)
                        .await;
                    }

                    current_position += 10.0;
                }
            }
            "key_press" => {
                let step_row = sqlx::query(
                    "INSERT INTO automation_steps (automation_id, position, step_type, post_delay_ms) VALUES ($1, $2, 'key_press', 0) RETURNING id",
                )
                .bind(new_automation_id)
                .bind(current_position)
                .fetch_one(&mut *tx)
                .await;

                if let Ok(r) = step_row {
                    let step_id: i64 = r.get("id");
                    last_created_step_id = Some(step_id);

                    let combo = key_combo.as_deref().unwrap_or("Enter");
                    let _ = sqlx::query(
                        "INSERT INTO step_key_presses (step_id, key_combo) VALUES ($1, $2)",
                    )
                    .bind(step_id)
                    .bind(combo)
                    .execute(&mut *tx)
                    .await;

                    if let Some(ref key) = screenshot_object_key {
                        let _ = sqlx::query(
                            "INSERT INTO step_screenshots (step_id, object_storage_key, width, height) VALUES ($1, $2, 1920, 1080) ON CONFLICT (step_id) DO UPDATE SET object_storage_key = EXCLUDED.object_storage_key",
                        )
                        .bind(step_id)
                        .bind(key)
                        .execute(&mut *tx)
                        .await;
                    }

                    current_position += 10.0;
                }
            }
            "screenshot" => {
                if let Some(ref key) = screenshot_object_key {
                    if let Some(step_id) = last_created_step_id {
                        let _ = sqlx::query(
                            "INSERT INTO step_screenshots (step_id, object_storage_key, width, height) VALUES ($1, $2, 1920, 1080) ON CONFLICT (step_id) DO UPDATE SET object_storage_key = EXCLUDED.object_storage_key",
                        )
                        .bind(step_id)
                        .bind(key)
                        .execute(&mut *tx)
                        .await;
                    }
                }
            }
            _ => {}
        }
    }

    let _ = sqlx::query(
        "UPDATE recording_sessions SET status = 'imported', resulting_automation_id = $1 WHERE id = $2",
    )
    .bind(new_automation_id)
    .bind(id)
    .execute(&mut *tx)
    .await;

    if tx.commit().await.is_ok() {
        let _ = log_audit(
            &state.db,
            Some(user.id),
            "convert_recording_session",
            "automation",
            Some(new_automation_id),
            Some(serde_json::json!({
                "recording_session_id": id,
                "automation_name": auto_name,
                "excluded_event_ids": form.exclude_event_ids,
            })),
        )
        .await;
    }

    Redirect::to(&format!("/automations/{}", new_automation_id)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_session_detail_methods() {
        let session = RecordingSessionDetail {
            id: 1,
            worker_id: 2,
            worker_display_name: "Worker 1".to_string(),
            worker_hostname: "worker-1.local".to_string(),
            started_by_user_id: 3,
            started_by_display_name: "Admin".to_string(),
            status: "recording".to_string(),
            resulting_automation_id: None,
            started_at: Utc::now(),
            ended_at: None,
        };

        assert_eq!(session.status_badge_class(), "badge-warning");
        assert_eq!(session.formatted_ended_at(), "In progress");
    }

    #[test]
    fn test_recording_event_detail_description() {
        let event_click = RecordingEventDetail {
            id: 10,
            recording_session_id: 1,
            sequence_number: 1,
            event_type: "mouse_click".to_string(),
            x: Some(824),
            y: Some(391),
            button: Some("left".to_string()),
            key_combo: None,
            screenshot_object_key: None,
            captured_at: Utc::now(),
            magnifier: None,
        };

        assert_eq!(event_click.description(), "Mouse Click at (824, 391) [left]");

        let event_key = RecordingEventDetail {
            id: 11,
            recording_session_id: 1,
            sequence_number: 2,
            event_type: "key_press".to_string(),
            x: None,
            y: None,
            button: None,
            key_combo: Some("ctrl+c".to_string()),
            screenshot_object_key: None,
            captured_at: Utc::now(),
            magnifier: None,
        };

        assert_eq!(event_key.description(), "Key Press: ctrl+c");

        let event_shot = RecordingEventDetail {
            id: 12,
            recording_session_id: 1,
            sequence_number: 3,
            event_type: "screenshot".to_string(),
            x: None,
            y: None,
            button: None,
            key_combo: None,
            screenshot_object_key: Some("recordings/1/shot.png".to_string()),
            captured_at: Utc::now(),
            magnifier: None,
        };

        assert_eq!(event_shot.description(), "Screen Capture");
    }

    #[test]
    fn test_recording_session_status_badge_classes() {
        let mut session = RecordingSessionDetail {
            id: 1,
            worker_id: 2,
            worker_display_name: "Worker 1".to_string(),
            worker_hostname: "worker-1".to_string(),
            started_by_user_id: 3,
            started_by_display_name: "User".to_string(),
            status: "recording".to_string(),
            resulting_automation_id: None,
            started_at: Utc::now(),
            ended_at: None,
        };

        assert_eq!(session.status_badge_class(), "badge-warning");

        session.status = "completed".to_string();
        assert_eq!(session.status_badge_class(), "badge-success");

        session.status = "imported".to_string();
        assert_eq!(session.status_badge_class(), "badge-info");

        session.status = "discarded".to_string();
        assert_eq!(session.status_badge_class(), "badge-danger");

        session.status = "unknown".to_string();
        assert_eq!(session.status_badge_class(), "badge-neutral");
    }

    #[test]
    fn test_convert_recording_form_deserialization() {
        let json_data = r#"{
            "automation_name": "My Recorded Flow",
            "automation_description": "Imported from worker 1",
            "exclude_event_ids": [10, 12]
        }"#;

        let form: ConvertRecordingForm = serde_json::from_str(json_data).expect("Failed to deserialize ConvertRecordingForm");
        assert_eq!(form.automation_name, "My Recorded Flow");
        assert_eq!(form.automation_description.as_deref(), Some("Imported from worker 1"));
        assert_eq!(form.exclude_event_ids, vec![10, 12]);
    }

    #[test]
    fn test_recording_templates_rendering() {
        use crate::auth::UserRole;

        let user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf_token".to_string(),
        };

        let worker = WorkerDetail {
            id: 5,
            hostname: "worker-05.local".to_string(),
            display_name: "Worker 5".to_string(),
            status: "online".to_string(),
            last_heartbeat_at: Some(Utc::now()),
            screen_width: Some(1920),
            screen_height: Some(1080),
            os_info: Some("Linux".to_string()),
            agent_version: Some("1.0.0".to_string()),
            created_at: Utc::now(),
            groups: vec![],
            registration_token: None,
        };

        let start_tmpl = RecordingStartTemplate {
            user: user.clone(),
            worker,
            error: None,
        };
        let start_html = start_tmpl.render().expect("Start template should render");
        assert!(start_html.contains("Worker 5"));
        assert!(start_html.contains("worker-05.local"));
        assert!(start_html.contains("Start Recording Now"));

        let session = RecordingSessionDetail {
            id: 42,
            worker_id: 5,
            worker_display_name: "Worker 5".to_string(),
            worker_hostname: "worker-05.local".to_string(),
            started_by_user_id: 1,
            started_by_display_name: "Admin User".to_string(),
            status: "recording".to_string(),
            resulting_automation_id: None,
            started_at: Utc::now(),
            ended_at: None,
        };

        let status_tmpl = RecordingStatusTemplate {
            user: user.clone(),
            session: session.clone(),
            event_count: 7,
            error: None,
        };
        let status_html = status_tmpl.render().expect("Status template should render");
        assert!(status_html.contains("Recording Session #42"));
        assert!(status_html.contains("7"));
        assert!(status_html.contains("http-equiv=\"refresh\""));

        let review_tmpl = RecordingReviewTemplate {
            user,
            session,
            events: vec![],
            default_automation_name: "Recorded Automation - 2025".to_string(),
            error: None,
        };
        let review_html = review_tmpl.render().expect("Review template should render");
        assert!(review_html.contains("Recorded Automation - 2025"));
        assert!(review_html.contains("Convert to Draft Automation"));
    }
}
