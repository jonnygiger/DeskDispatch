use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{auth::AuthWorker, AppState};

#[derive(Debug, Deserialize)]
pub struct RegisterWorkerRequest {
    pub registration_token: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterWorkerResponse {
    pub status: String,
    pub worker_id: i64,
    pub hostname: String,
    pub display_name: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub status: Option<String>,
    pub current_task_run_id: Option<i64>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HeartbeatResponse {
    pub status: String,
    pub cancel_requested: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAssignmentResponse {
    None,
    ExecuteAutomation {
        task_run_id: i64,
        automation: serde_json::Value,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunResponse {
    pub task_run_id: i64,
    pub status: String,
    pub automation: serde_json::Value,
}

pub async fn fetch_full_automation_json(
    conn: &mut sqlx::PgConnection,
    automation_id: i64,
) -> Result<serde_json::Value, sqlx::Error> {
    let auto_row = sqlx::query(
        "SELECT id, name, description, status FROM automations WHERE id = $1",
    )
    .bind(automation_id)
    .fetch_optional(&mut *conn)
    .await?;

    let (id, name, description, status) = match auto_row {
        Some(row) => (
            row.get::<i64, _>("id"),
            row.get::<String, _>("name"),
            row.get::<String, _>("description"),
            row.get::<String, _>("status"),
        ),
        None => {
            return Ok(serde_json::json!({
                "id": automation_id,
            }));
        }
    };

    let param_rows = sqlx::query(
        "SELECT name, param_type, default_value FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC",
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut parameters_map = serde_json::Map::new();
    for p in param_rows {
        let pname: String = p.get("name");
        let ptype: String = p.get("param_type");
        let pval_str: String = p.get("default_value");

        let val = match ptype.as_str() {
            "int" => pval_str.parse::<i64>().map(serde_json::Value::from).unwrap_or(serde_json::Value::String(pval_str)),
            "bool" => {
                let lower = pval_str.to_lowercase();
                serde_json::Value::Bool(matches!(lower.as_str(), "true" | "1" | "yes"))
            }
            _ => serde_json::Value::String(pval_str),
        };
        parameters_map.insert(pname, val);
    }

    let var_rows = sqlx::query(
        "SELECT id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY id ASC",
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut variables_list = Vec::new();
    for v in var_rows {
        variables_list.push(serde_json::json!({
            "id": v.get::<i64, _>("id"),
            "name": v.get::<String, _>("name"),
            "var_type": v.get::<String, _>("var_type"),
            "description": v.get::<String, _>("description"),
        }));
    }

    let step_rows = sqlx::query(
        "SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut steps_list = Vec::new();
    for step in step_rows {
        let step_id: i64 = step.get("id");
        let step_type: String = step.get("step_type");
        let label: Option<String> = step.get("label");
        let post_delay_ms: i32 = step.get("post_delay_ms");

        let mut step_json = serde_json::json!({
            "id": step_id,
            "type": step_type,
            "label": label,
            "post_delay_ms": post_delay_ms,
        });

        match step_type.as_str() {
            "mouse_click" => {
                let mc_row = sqlx::query(
                    "SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1",
                )
                .bind(step_id)
                .fetch_optional(&mut *conn)
                .await?;

                if let Some(r) = mc_row {
                    step_json["x"] = serde_json::json!(r.get::<Option<i32>, _>("x"));
                    step_json["y"] = serde_json::json!(r.get::<Option<i32>, _>("y"));
                    step_json["x_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("x_variable_id"));
                    step_json["y_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("y_variable_id"));
                    step_json["button"] = serde_json::json!(r.get::<String, _>("button"));
                    step_json["click_type"] = serde_json::json!(r.get::<String, _>("click_type"));
                }
            }
            "key_press" => {
                let kp_row = sqlx::query(
                    "SELECT key_combo FROM step_key_presses WHERE step_id = $1",
                )
                .bind(step_id)
                .fetch_optional(&mut *conn)
                .await?;

                if let Some(r) = kp_row {
                    step_json["key_combo"] = serde_json::json!(r.get::<String, _>("key_combo"));
                }
            }
            "find_pixel_rgb" => {
                let fp_row = sqlx::query(
                    "SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1",
                )
                .bind(step_id)
                .fetch_optional(&mut *conn)
                .await?;

                if let Some(r) = fp_row {
                    step_json["x"] = serde_json::json!(r.get::<i32, _>("x"));
                    step_json["y"] = serde_json::json!(r.get::<i32, _>("y"));
                    step_json["output_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("output_variable_id"));
                }
            }
            "find_bitmap" => {
                let fb_row = sqlx::query(
                    r#"
                    SELECT fb.reference_bitmap_id, fb.search_x, fb.search_y, fb.search_width, fb.search_height,
                           fb.match_threshold, fb.output_found_variable_id, fb.output_x_variable_id, fb.output_y_variable_id,
                           b.object_storage_key AS reference_bitmap_key
                    FROM step_find_bitmap fb
                    LEFT JOIN bitmaps b ON fb.reference_bitmap_id = b.id
                    WHERE fb.step_id = $1
                    "#,
                )
                .bind(step_id)
                .fetch_optional(&mut *conn)
                .await?;

                if let Some(r) = fb_row {
                    step_json["reference_bitmap_id"] = serde_json::json!(r.get::<i64, _>("reference_bitmap_id"));
                    step_json["reference_bitmap_key"] = serde_json::json!(r.get::<Option<String>, _>("reference_bitmap_key"));
                    step_json["search_x"] = serde_json::json!(r.get::<Option<i32>, _>("search_x"));
                    step_json["search_y"] = serde_json::json!(r.get::<Option<i32>, _>("search_y"));
                    step_json["search_width"] = serde_json::json!(r.get::<Option<i32>, _>("search_width"));
                    step_json["search_height"] = serde_json::json!(r.get::<Option<i32>, _>("search_height"));
                    step_json["match_threshold"] = serde_json::json!(r.get::<f32, _>("match_threshold"));
                    step_json["output_found_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("output_found_variable_id"));
                    step_json["output_x_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("output_x_variable_id"));
                    step_json["output_y_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("output_y_variable_id"));
                }
            }
            "branch" => {
                let br_row = sqlx::query(
                    r#"
                    SELECT sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
                           sb.reference_bitmap_id, sb.search_x, sb.search_y, sb.search_width, sb.search_height, sb.match_threshold,
                           sb.on_match_step_id, sb.on_no_match_step_id,
                           b.object_storage_key AS reference_bitmap_key
                    FROM step_branches sb
                    LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
                    WHERE sb.step_id = $1
                    "#,
                )
                .bind(step_id)
                .fetch_optional(&mut *conn)
                .await?;

                if let Some(r) = br_row {
                    let cond_type: String = r.get("condition_type");
                    let condition_obj = if cond_type == "pixel_rgb" {
                        let exp_r: i16 = r.get::<Option<i16>, _>("expected_r").unwrap_or(0);
                        let exp_g: i16 = r.get::<Option<i16>, _>("expected_g").unwrap_or(0);
                        let exp_b: i16 = r.get::<Option<i16>, _>("expected_b").unwrap_or(0);
                        serde_json::json!({
                            "type": "pixel_rgb",
                            "x": r.get::<Option<i32>, _>("x"),
                            "y": r.get::<Option<i32>, _>("y"),
                            "expected_r": exp_r,
                            "expected_g": exp_g,
                            "expected_b": exp_b,
                            "expected_rgb": [exp_r, exp_g, exp_b],
                            "tolerance": r.get::<Option<i16>, _>("tolerance").unwrap_or(0),
                        })
                    } else {
                        serde_json::json!({
                            "type": "bitmap",
                            "reference_bitmap_id": r.get::<Option<i64>, _>("reference_bitmap_id"),
                            "reference_bitmap_key": r.get::<Option<String>, _>("reference_bitmap_key"),
                            "search_x": r.get::<Option<i32>, _>("search_x"),
                            "search_y": r.get::<Option<i32>, _>("search_y"),
                            "search_width": r.get::<Option<i32>, _>("search_width"),
                            "search_height": r.get::<Option<i32>, _>("search_height"),
                            "match_threshold": r.get::<Option<f32>, _>("match_threshold"),
                        })
                    };

                    step_json["condition"] = condition_obj;
                    step_json["on_match_step_id"] = serde_json::json!(r.get::<Option<i64>, _>("on_match_step_id"));
                    step_json["on_no_match_step_id"] = serde_json::json!(r.get::<Option<i64>, _>("on_no_match_step_id"));
                }
            }
            _ => {}
        }

        steps_list.push(step_json);
    }

    Ok(serde_json::json!({
        "id": id,
        "name": name,
        "description": description,
        "status": status,
        "parameters": parameters_map,
        "variables": variables_list,
        "steps": steps_list,
    }))
}

pub async fn get_next_assignment_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(worker_id = worker.id, "Failed to begin transaction for next assignment: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error".to_string(),
                }),
            )
                .into_response();
        }
    };

    let select_res = sqlx::query(
        r#"
        SELECT tr.id AS task_run_id, tr.automation_id
        FROM task_runs tr
        LEFT JOIN schedules s ON tr.schedule_id = s.id
        WHERE tr.status = 'queued'
          AND (
            tr.schedule_id IS NULL
            OR s.worker_group_id IS NULL
            OR EXISTS (
              SELECT 1 FROM worker_group_members wgm
              WHERE wgm.worker_id = $1 AND wgm.group_id = s.worker_group_id
            )
          )
        ORDER BY tr.queued_at ASC, tr.id ASC
        LIMIT 1
        FOR UPDATE OF tr SKIP LOCKED
        "#,
    )
    .bind(worker.id)
    .fetch_optional(&mut *tx)
    .await;

    match select_res {
        Ok(Some(row)) => {
            let task_run_id: i64 = row.get("task_run_id");
            let automation_id: i64 = row.get("automation_id");

            let update_res = sqlx::query(
                r#"
                UPDATE task_runs
                SET status = 'running',
                    worker_id = $1,
                    started_at = now()
                WHERE id = $2
                "#,
            )
            .bind(worker.id)
            .bind(task_run_id)
            .execute(&mut *tx)
            .await;

            if let Err(e) = update_res {
                tracing::error!(
                    worker_id = worker.id,
                    task_run_id = task_run_id,
                    "Failed to update task run status to running: {}",
                    e
                );
                let _ = tx.rollback().await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to claim task run".to_string(),
                    }),
                )
                    .into_response();
            }

            let automation_json = match fetch_full_automation_json(&mut *tx, automation_id).await {
                Ok(json) => json,
                Err(e) => {
                    tracing::error!(
                        worker_id = worker.id,
                        automation_id = automation_id,
                        "Failed to fetch automation json payload: {}",
                        e
                    );
                    let _ = tx.rollback().await;
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to load automation details".to_string(),
                        }),
                    )
                        .into_response();
                }
            };

            if let Err(e) = tx.commit().await {
                tracing::error!(
                    worker_id = worker.id,
                    task_run_id = task_run_id,
                    "Failed to commit transaction claiming task run: {}",
                    e
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to commit task run claim".to_string(),
                    }),
                )
                    .into_response();
            }

            tracing::info!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                automation_id = automation_id,
                "Claimed queued task run for worker"
            );

            (
                StatusCode::OK,
                Json(NextAssignmentResponse::ExecuteAutomation {
                    task_run_id,
                    automation: automation_json,
                }),
            )
                .into_response()
        }
        Ok(None) => {
            let _ = tx.commit().await;
            (StatusCode::OK, Json(NextAssignmentResponse::None)).into_response()
        }
        Err(e) => {
            tracing::error!(worker_id = worker.id, "Error fetching next task run assignment: {}", e);
            let _ = tx.rollback().await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error querying next assignment".to_string(),
                }),
            )
                .into_response()
        }
    }
}

pub async fn get_task_run_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
) -> impl IntoResponse {
    let mut conn = match state.db.acquire().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Failed to acquire db connection for get_task_run: {}",
                e
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error".to_string(),
                }),
            )
                .into_response();
        }
    };

    let row = sqlx::query(
        "SELECT id, automation_id, status FROM task_runs WHERE id = $1",
    )
    .bind(task_run_id)
    .fetch_optional(&mut *conn)
    .await;

    match row {
        Ok(Some(row)) => {
            let id: i64 = row.get("id");
            let automation_id: i64 = row.get("automation_id");
            let status: String = row.get("status");

            let automation_json = match fetch_full_automation_json(&mut *conn, automation_id).await {
                Ok(json) => json,
                Err(e) => {
                    tracing::error!(
                        worker_id = worker.id,
                        automation_id = automation_id,
                        task_run_id = task_run_id,
                        "Failed to fetch automation json payload for task run: {}",
                        e
                    );
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to load automation details".to_string(),
                        }),
                    )
                        .into_response();
                }
            };

            (
                StatusCode::OK,
                Json(TaskRunResponse {
                    task_run_id: id,
                    status,
                    automation: automation_json,
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Task run {} not found", task_run_id),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Error querying task run: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error querying task run".to_string(),
                }),
            )
                .into_response()
        }
    }
}

pub fn is_valid_worker_status(status: &str) -> bool {
    matches!(status, "offline" | "online" | "busy" | "error")
}

pub async fn post_heartbeat_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    let new_status = if let Some(ref status_str) = payload.status {
        let trimmed = status_str.trim();
        if !is_valid_worker_status(trimmed) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid status '{}'. Expected one of: offline, online, busy, error", status_str),
                }),
            )
                .into_response();
        }
        trimmed.to_string()
    } else {
        // Default to 'online' if worker is sending a heartbeat without explicit status change, or keep current status if busy?
        // Spec: "Body: {status, current_task_run_id?}"
        // If status is omitted, we keep worker's existing status unless it was offline/error where online is default, or just fallback to current status/online.
        if is_valid_worker_status(&worker.status) {
            worker.status.clone()
        } else {
            "online".to_string()
        }
    };

    let update_result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET
            last_heartbeat_at = now(),
            status = $1,
            screen_width = COALESCE($2, screen_width),
            screen_height = COALESCE($3, screen_height),
            os_info = COALESCE($4, os_info),
            agent_version = COALESCE($5, agent_version)
        WHERE id = $6
        "#,
    )
    .bind(&new_status)
    .bind(payload.screen_width)
    .bind(payload.screen_height)
    .bind(payload.os_info.as_deref())
    .bind(payload.agent_version.as_deref())
    .bind(worker.id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_result {
        tracing::error!(worker_id = worker.id, "Failed to update heartbeat: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Database error during heartbeat".to_string(),
            }),
        )
            .into_response();
    }

    let mut cancel_requested = false;

    if let Some(task_run_id) = payload.current_task_run_id {
        let run_status_res: Result<Option<String>, _> = sqlx::query_scalar(
            r#"
            SELECT status FROM task_runs WHERE id = $1 AND worker_id = $2
            "#,
        )
        .bind(task_run_id)
        .bind(worker.id)
        .fetch_optional(&state.db)
        .await;

        match run_status_res {
            Ok(Some(status)) => {
                if status == "cancelled" {
                    cancel_requested = true;
                }
            }
            Ok(None) => {
                // If run doesn't exist or isn't assigned to this worker, flag cancellation
                cancel_requested = true;
            }
            Err(e) => {
                tracing::error!(
                    worker_id = worker.id,
                    task_run_id = task_run_id,
                    "Error querying task run status for heartbeat cancel flag: {}",
                    e
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(HeartbeatResponse {
            status: "success".to_string(),
            cancel_requested,
        }),
    )
        .into_response()
}

pub async fn post_register_worker_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterWorkerRequest>,
) -> impl IntoResponse {
    let token = payload
        .registration_token
        .or(payload.token)
        .map(|t| t.trim().to_string())
        .unwrap_or_default();

    if token.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "registration_token is required".to_string(),
            }),
        )
            .into_response();
    }

    // Generate a 256-bit (64 hex characters) random API key
    let api_key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());

    // Compute SHA-256 hash of the bearer API key
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let api_key_hash: Vec<u8> = hasher.finalize().to_vec();

    let row_result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET api_key_hash = $1, registration_token = NULL
        WHERE registration_token = $2 AND registration_token IS NOT NULL
        RETURNING id, hostname, display_name
        "#,
    )
    .bind(&api_key_hash)
    .bind(&token)
    .fetch_optional(&state.db)
    .await;

    match row_result {
        Ok(Some(row)) => {
            let worker_id: i64 = row.get("id");
            let hostname: String = row.get("hostname");
            let display_name: String = row.get("display_name");

            tracing::info!(
                worker_id = worker_id,
                hostname = %hostname,
                "Task worker PC registered successfully"
            );

            (
                StatusCode::OK,
                Json(RegisterWorkerResponse {
                    status: "success".to_string(),
                    worker_id,
                    hostname,
                    display_name,
                    api_key,
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid or expired registration token".to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to register worker PC: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error during registration".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_worker_request_deserialization() {
        let json_data = r#"{"registration_token": "token-123"}"#;
        let req: RegisterWorkerRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize registration_token");
        assert_eq!(req.registration_token.as_deref(), Some("token-123"));
        assert_eq!(req.token, None);

        let json_data_alt = r#"{"token": "token-456"}"#;
        let req_alt: RegisterWorkerRequest =
            serde_json::from_str(json_data_alt).expect("Failed to deserialize token");
        assert_eq!(req_alt.registration_token, None);
        assert_eq!(req_alt.token.as_deref(), Some("token-456"));
    }

    #[test]
    fn test_api_key_generation_and_sha256_hash() {
        let api_key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        assert_eq!(api_key.len(), 64);

        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        let hash: Vec<u8> = hasher.finalize().to_vec();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_is_valid_worker_status() {
        assert!(is_valid_worker_status("offline"));
        assert!(is_valid_worker_status("online"));
        assert!(is_valid_worker_status("busy"));
        assert!(is_valid_worker_status("error"));

        assert!(!is_valid_worker_status("invalid"));
        assert!(!is_valid_worker_status(""));
        assert!(!is_valid_worker_status("UNKNOWN"));
    }

    #[test]
    fn test_heartbeat_request_deserialization() {
        let json_data = r#"{
            "status": "online",
            "current_task_run_id": 123,
            "screen_width": 1920,
            "screen_height": 1080,
            "os_info": "Linux x86_64",
            "agent_version": "1.0.0"
        }"#;

        let req: HeartbeatRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize HeartbeatRequest");
        assert_eq!(req.status.as_deref(), Some("online"));
        assert_eq!(req.current_task_run_id, Some(123));
        assert_eq!(req.screen_width, Some(1920));
        assert_eq!(req.screen_height, Some(1080));
        assert_eq!(req.os_info.as_deref(), Some("Linux x86_64"));
        assert_eq!(req.agent_version.as_deref(), Some("1.0.0"));

        let empty_json = r#"{}"#;
        let empty_req: HeartbeatRequest =
            serde_json::from_str(empty_json).expect("Failed to deserialize empty HeartbeatRequest");
        assert_eq!(empty_req.status, None);
        assert_eq!(empty_req.current_task_run_id, None);
    }

    #[test]
    fn test_heartbeat_response_serialization() {
        let resp = HeartbeatResponse {
            status: "success".to_string(),
            cancel_requested: true,
        };

        let json_str = serde_json::to_string(&resp).expect("Failed to serialize HeartbeatResponse");
        assert!(json_str.contains(r#""status":"success""#));
        assert!(json_str.contains(r#""cancel_requested":true"#));
    }

    #[test]
    fn test_next_assignment_response_serialization() {
        let resp = NextAssignmentResponse::None;
        let json_str = serde_json::to_string(&resp).expect("Failed to serialize NextAssignmentResponse");
        assert_eq!(json_str, r#"{"type":"none"}"#);

        let deserialized: NextAssignmentResponse = serde_json::from_str(r#"{"type":"none"}"#)
            .expect("Failed to deserialize NextAssignmentResponse::None");
        assert_eq!(deserialized, NextAssignmentResponse::None);

        let exec_resp = NextAssignmentResponse::ExecuteAutomation {
            task_run_id: 4821,
            automation: serde_json::json!({
                "id": 12,
                "parameters": { "click_tolerance": 8 }
            }),
        };
        let exec_json = serde_json::to_string(&exec_resp).expect("Failed to serialize ExecuteAutomation");
        assert!(exec_json.contains(r#""type":"execute_automation""#));
        assert!(exec_json.contains(r#""task_run_id":4821"#));
        assert!(exec_json.contains(r#""automation":{"id":12,"parameters":{"click_tolerance":8}}"#));

        let exec_deserialized: NextAssignmentResponse = serde_json::from_str(&exec_json)
            .expect("Failed to deserialize ExecuteAutomation");
        assert_eq!(exec_deserialized, exec_resp);
    }

    #[test]
    fn test_execute_automation_full_payload_serialization() {
        let full_automation = serde_json::json!({
            "id": 12,
            "name": "Sample Automation",
            "description": "Automation for testing dispatch",
            "status": "active",
            "parameters": {
                "click_tolerance": 8,
                "auto_retry": true,
                "label_prefix": "Test"
            },
            "variables": [
                {
                    "id": 1,
                    "name": "bg_color",
                    "var_type": "color",
                    "description": "Background color"
                }
            ],
            "steps": [
                {
                    "id": 501,
                    "type": "mouse_click",
                    "label": "Click main button",
                    "post_delay_ms": 500,
                    "x": 824,
                    "y": 391,
                    "x_variable_id": null,
                    "y_variable_id": null,
                    "button": "left",
                    "click_type": "single"
                },
                {
                    "id": 502,
                    "type": "branch",
                    "label": null,
                    "post_delay_ms": 0,
                    "condition": {
                        "type": "pixel_rgb",
                        "x": 824,
                        "y": 391,
                        "expected_r": 40,
                        "expected_g": 180,
                        "expected_b": 60,
                        "expected_rgb": [40, 180, 60],
                        "tolerance": 10
                    },
                    "on_match_step_id": 509,
                    "on_no_match_step_id": 503
                }
            ]
        });

        let resp = NextAssignmentResponse::ExecuteAutomation {
            task_run_id: 4821,
            automation: full_automation.clone(),
        };

        let json_str = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(json_str.contains(r#""type":"execute_automation""#));
        assert!(json_str.contains(r#""task_run_id":4821"#));
        assert!(json_str.contains(r#""name":"Sample Automation""#));
        assert!(json_str.contains(r#""click_tolerance":8"#));
        assert!(json_str.contains(r#""expected_rgb":[40,180,60]"#));

        let deserialized: NextAssignmentResponse =
            serde_json::from_str(&json_str).expect("Deserialization failed");
        assert_eq!(deserialized, resp);
    }

    #[test]
    fn test_task_run_response_serialization() {
        let resp = TaskRunResponse {
            task_run_id: 4821,
            status: "running".to_string(),
            automation: serde_json::json!({
                "id": 12,
                "name": "Sample Automation",
                "steps": []
            }),
        };

        let json_str = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(json_str.contains(r#""task_run_id":4821"#));
        assert!(json_str.contains(r#""status":"running""#));
        assert!(json_str.contains(r#""name":"Sample Automation""#));

        let deserialized: TaskRunResponse =
            serde_json::from_str(&json_str).expect("Deserialization failed");
        assert_eq!(deserialized, resp);
    }
}
