use std::collections::HashMap;

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
    StartRecording {
        recording_session_id: i64,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunResponse {
    pub task_run_id: i64,
    pub status: String,
    pub automation: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct VariableUpdateItem {
    pub variable_id: i64,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct StepResultRequest {
    pub step_id: i64,
    pub result: String,
    pub captured_r: Option<i16>,
    pub captured_g: Option<i16>,
    pub captured_b: Option<i16>,
    pub captured_rgb: Option<Vec<i16>>,
    pub captured_found: Option<bool>,
    pub captured_x: Option<i32>,
    pub captured_y: Option<i32>,
    pub captured_xy: Option<Vec<i32>>,
    pub screenshot_object_key: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub variable_updates: Option<Vec<VariableUpdateItem>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct StepResultResponse {
    pub status: String,
    pub step_result_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct CompleteTaskRunRequest {
    pub status: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CompleteTaskRunResponse {
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenshotUploadUrlResponse {
    pub upload_url: String,
    pub object_key: String,
}

#[derive(Debug, Deserialize)]
pub struct RecordingEventItem {
    pub sequence_number: i32,
    pub event_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub button: Option<String>,
    pub key_combo: Option<String>,
    pub screenshot_object_key: Option<String>,
    pub captured_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct PostRecordingEventsRequest {
    pub events: Vec<RecordingEventItem>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PostRecordingEventsResponse {
    pub status: String,
    pub count: usize,
    pub stop_requested: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WorkerStopRecordingResponse {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitScreenshotRequest {
    pub step_id: Option<i64>,
    pub object_key: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CommitScreenshotResponse {
    pub status: String,
    pub object_key: String,
}

#[tracing::instrument(skip(conn))]
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

    // Bolt Optimization: Batch fetch step subtype details to resolve N+1 query bottleneck.
    // Bulk-fetch all step subtype details for this automation into HashMaps in fixed O(1) bulk queries
    // instead of issuing O(N) database queries in a loop.

    let mouse_clicks_rows = sqlx::query(
        r#"
        SELECT smc.step_id, smc.x, smc.y, smc.x_variable_id, smc.y_variable_id, smc.button, smc.click_type
        FROM step_mouse_clicks smc
        JOIN automation_steps s ON smc.step_id = s.id
        WHERE s.automation_id = $1
        "#,
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut mouse_clicks_map = HashMap::new();
    for r in mouse_clicks_rows {
        let step_id: i64 = r.get("step_id");
        mouse_clicks_map.insert(step_id, r);
    }

    let key_presses_rows = sqlx::query(
        r#"
        SELECT skp.step_id, skp.key_combo
        FROM step_key_presses skp
        JOIN automation_steps s ON skp.step_id = s.id
        WHERE s.automation_id = $1
        "#,
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut key_presses_map = HashMap::new();
    for r in key_presses_rows {
        let step_id: i64 = r.get("step_id");
        key_presses_map.insert(step_id, r);
    }

    let find_pixel_rows = sqlx::query(
        r#"
        SELECT sfp.step_id, sfp.x, sfp.y, sfp.output_variable_id
        FROM step_find_pixel_rgb sfp
        JOIN automation_steps s ON sfp.step_id = s.id
        WHERE s.automation_id = $1
        "#,
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut find_pixel_map = HashMap::new();
    for r in find_pixel_rows {
        let step_id: i64 = r.get("step_id");
        find_pixel_map.insert(step_id, r);
    }

    let find_bitmap_rows = sqlx::query(
        r#"
        SELECT sfb.step_id, sfb.reference_bitmap_id, sfb.search_x, sfb.search_y, sfb.search_width, sfb.search_height,
               sfb.match_threshold, sfb.output_found_variable_id, sfb.output_x_variable_id, sfb.output_y_variable_id,
               b.object_storage_key AS reference_bitmap_key
        FROM step_find_bitmap sfb
        JOIN automation_steps s ON sfb.step_id = s.id
        LEFT JOIN bitmaps b ON sfb.reference_bitmap_id = b.id
        WHERE s.automation_id = $1
        "#,
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut find_bitmap_map = HashMap::new();
    for r in find_bitmap_rows {
        let step_id: i64 = r.get("step_id");
        find_bitmap_map.insert(step_id, r);
    }

    let branch_rows = sqlx::query(
        r#"
        SELECT sb.step_id, sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
               sb.reference_bitmap_id, sb.search_x, sb.search_y, sb.search_width, sb.search_height, sb.match_threshold,
               sb.on_match_step_id, sb.on_no_match_step_id,
               b.object_storage_key AS reference_bitmap_key
        FROM step_branches sb
        JOIN automation_steps s ON sb.step_id = s.id
        LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
        WHERE s.automation_id = $1
        "#,
    )
    .bind(automation_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut branch_map = HashMap::new();
    for r in branch_rows {
        let step_id: i64 = r.get("step_id");
        branch_map.insert(step_id, r);
    }

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
                if let Some(r) = mouse_clicks_map.get(&step_id) {
                    step_json["x"] = serde_json::json!(r.get::<Option<i32>, _>("x"));
                    step_json["y"] = serde_json::json!(r.get::<Option<i32>, _>("y"));
                    step_json["x_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("x_variable_id"));
                    step_json["y_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("y_variable_id"));
                    step_json["button"] = serde_json::json!(r.get::<String, _>("button"));
                    step_json["click_type"] = serde_json::json!(r.get::<String, _>("click_type"));
                }
            }
            "key_press" => {
                if let Some(r) = key_presses_map.get(&step_id) {
                    step_json["key_combo"] = serde_json::json!(r.get::<String, _>("key_combo"));
                }
            }
            "find_pixel_rgb" => {
                if let Some(r) = find_pixel_map.get(&step_id) {
                    step_json["x"] = serde_json::json!(r.get::<i32, _>("x"));
                    step_json["y"] = serde_json::json!(r.get::<i32, _>("y"));
                    step_json["output_variable_id"] = serde_json::json!(r.get::<Option<i64>, _>("output_variable_id"));
                }
            }
            "find_bitmap" => {
                if let Some(r) = find_bitmap_map.get(&step_id) {
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
                if let Some(r) = branch_map.get(&step_id) {
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

#[tracing::instrument(skip(worker, state))]
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

    let active_recording: Option<i64> = match sqlx::query_scalar(
        "SELECT id FROM recording_sessions WHERE worker_id = $1 AND status = 'recording' ORDER BY id ASC LIMIT 1",
    )
    .bind(worker.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(rec) => rec,
        Err(e) => {
            tracing::error!(worker_id = worker.id, "Error checking active recording sessions: {}", e);
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error checking recording sessions".to_string(),
                }),
            )
                .into_response();
        }
    };

    if let Some(recording_session_id) = active_recording {
        let _ = tx.commit().await;
        tracing::info!(
            worker_id = worker.id,
            recording_session_id = recording_session_id,
            "Dispatched start_recording instruction to worker"
        );
        return (
            StatusCode::OK,
            Json(NextAssignmentResponse::StartRecording {
                recording_session_id,
            }),
        )
            .into_response();
    }

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

pub fn is_valid_step_result(result: &str) -> bool {
    matches!(
        result,
        "success" | "failed" | "branch_matched" | "branch_not_matched"
    )
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_step_result_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    Json(payload): Json<StepResultRequest>,
) -> impl IntoResponse {
    let result_str = payload.result.trim();
    if !is_valid_step_result(result_str) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid step result '{}'. Expected one of: success, failed, branch_matched, branch_not_matched",
                    payload.result
                ),
            }),
        )
            .into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Failed to begin transaction for step-result: {}",
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

    let run_exists: Option<(i64, String)> = match sqlx::query_as(
        "SELECT id, status FROM task_runs WHERE id = $1 AND worker_id = $2",
    )
    .bind(task_run_id)
    .bind(worker.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Error checking task run ownership: {}",
                e
            );
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error checking task run".to_string(),
                }),
            )
                .into_response();
        }
    };

    if run_exists.is_none() {
        let _ = tx.rollback().await;
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Running task run {} not found for this worker", task_run_id),
            }),
        )
            .into_response();
    }

    let captured_r = payload.captured_r.or_else(|| {
        payload
            .captured_rgb
            .as_ref()
            .and_then(|v| v.get(0).copied())
    });
    let captured_g = payload.captured_g.or_else(|| {
        payload
            .captured_rgb
            .as_ref()
            .and_then(|v| v.get(1).copied())
    });
    let captured_b = payload.captured_b.or_else(|| {
        payload
            .captured_rgb
            .as_ref()
            .and_then(|v| v.get(2).copied())
    });

    let captured_x = payload.captured_x.or_else(|| {
        payload
            .captured_xy
            .as_ref()
            .and_then(|v| v.get(0).copied())
    });
    let captured_y = payload.captured_y.or_else(|| {
        payload
            .captured_xy
            .as_ref()
            .and_then(|v| v.get(1).copied())
    });

    let completed_at = payload
        .completed_at
        .or(payload.timestamp)
        .unwrap_or_else(chrono::Utc::now);
    let started_at = payload.started_at.unwrap_or(completed_at);

    let step_result_id: i64 = match sqlx::query_scalar(
        r#"
        INSERT INTO task_run_steps (
            task_run_id, step_id, started_at, completed_at, result,
            captured_r, captured_g, captured_b, captured_found, captured_x, captured_y,
            screenshot_object_key
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING id
        "#,
    )
    .bind(task_run_id)
    .bind(payload.step_id)
    .bind(started_at)
    .bind(completed_at)
    .bind(result_str)
    .bind(captured_r)
    .bind(captured_g)
    .bind(captured_b)
    .bind(payload.captured_found)
    .bind(captured_x)
    .bind(captured_y)
    .bind(payload.screenshot_object_key.as_deref())
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                step_id = payload.step_id,
                "Failed to insert task_run_steps: {}",
                e
            );
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to record step result".to_string(),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = sqlx::query(
        "UPDATE task_runs SET current_step_id = $1 WHERE id = $2",
    )
    .bind(payload.step_id)
    .bind(task_run_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(
            worker_id = worker.id,
            task_run_id = task_run_id,
            step_id = payload.step_id,
            "Failed to update task_runs current_step_id: {}",
            e
        );
        let _ = tx.rollback().await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to update current step on task run".to_string(),
            }),
        )
            .into_response();
    }

    if let Some(updates) = payload.variable_updates {
        for item in updates {
            let val_str = match &item.value {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };

            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO task_run_variable_values (task_run_id, variable_id, value, set_at_step_id, set_at)
                VALUES ($1, $2, $3, $4, now())
                ON CONFLICT (task_run_id, variable_id) DO UPDATE
                SET value = EXCLUDED.value,
                    set_at_step_id = EXCLUDED.set_at_step_id,
                    set_at = EXCLUDED.set_at
                "#,
            )
            .bind(task_run_id)
            .bind(item.variable_id)
            .bind(val_str)
            .bind(payload.step_id)
            .execute(&mut *tx)
            .await
            {
                tracing::error!(
                    worker_id = worker.id,
                    task_run_id = task_run_id,
                    variable_id = item.variable_id,
                    "Failed to upsert variable value: {}",
                    e
                );
                let _ = tx.rollback().await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to record variable updates".to_string(),
                    }),
                )
                    .into_response();
            }
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(
            worker_id = worker.id,
            task_run_id = task_run_id,
            "Failed to commit transaction for step-result: {}",
            e
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to commit step result".to_string(),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(StepResultResponse {
            status: "success".to_string(),
            step_result_id,
        }),
    )
        .into_response()
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_complete_task_run_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    Json(payload): Json<CompleteTaskRunRequest>,
) -> impl IntoResponse {
    let final_status = payload.status.trim();
    if final_status != "succeeded" && final_status != "failed" {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid completion status '{}'. Expected 'succeeded' or 'failed'",
                    payload.status
                ),
            }),
        )
            .into_response();
    }

    let update_res = sqlx::query(
        r#"
        UPDATE task_runs
        SET status = $1,
            completed_at = now(),
            error_message = COALESCE($2, error_message)
        WHERE id = $3 AND worker_id = $4
        RETURNING id
        "#,
    )
    .bind(final_status)
    .bind(payload.error_message.as_deref())
    .bind(task_run_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await;

    match update_res {
        Ok(Some(_)) => {
            tracing::info!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                status = final_status,
                "Task run completed"
            );
            (
                StatusCode::OK,
                Json(CompleteTaskRunResponse {
                    status: "success".to_string(),
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Running task run {} not found for this worker", task_run_id),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Failed to update task run completion status: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error completing task run".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(pool))]
pub async fn sweep_stalled_task_runs(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE task_runs
        SET status = 'lost',
            completed_at = now(),
            error_message = COALESCE(error_message, 'Worker heartbeat lost (stalled execution)')
        WHERE status = 'running'
          AND (
            worker_id IS NULL
            OR EXISTS (
              SELECT 1 FROM task_worker_pcs w
              WHERE w.id = task_runs.worker_id
                AND (w.last_heartbeat_at IS NULL OR w.last_heartbeat_at < now() - INTERVAL '90 seconds')
            )
          )
        "#,
    )
    .execute(pool)
    .await?;

    let count = result.rows_affected();
    if count > 0 {
        tracing::info!(count = count, "Swept stalled task runs with silent heartbeats to lost status");
    }
    Ok(count)
}

#[tracing::instrument(skip(worker, state))]
pub async fn get_task_run_screenshot_upload_url_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
) -> impl IntoResponse {
    let run_exists: Option<i64> = match sqlx::query_scalar(
        "SELECT id FROM task_runs WHERE id = $1 AND worker_id = $2",
    )
    .bind(task_run_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Error checking task run for screenshot upload URL: {}",
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

    if run_exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Running task run {} not found for this worker", task_run_id),
            }),
        )
            .into_response();
    }

    let object_key = format!("runs/{}/step_{}.png", task_run_id, Uuid::new_v4());
    let storage = state.storage_service();

    match storage
        .generate_presigned_put_url(&object_key, std::time::Duration::from_secs(900))
        .await
    {
        Ok(upload_url) => (
            StatusCode::OK,
            Json(ScreenshotUploadUrlResponse {
                upload_url,
                object_key,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Failed to generate presigned upload URL: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to generate upload URL".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_recording_events_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(payload): Json<PostRecordingEventsRequest>,
) -> impl IntoResponse {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                "Failed to begin transaction for recording events: {}",
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

    let session_row: Option<(String,)> = match sqlx::query_as(
        "SELECT status FROM recording_sessions WHERE id = $1 AND worker_id = $2",
    )
    .bind(session_id)
    .bind(worker.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                "Error querying recording session: {}",
                e
            );
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error checking recording session".to_string(),
                }),
            )
                .into_response();
        }
    };

    let session_status = match session_row {
        Some((status,)) => status,
        None => {
            let _ = tx.rollback().await;
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Recording session {} not found for this worker", session_id),
                }),
            )
                .into_response();
        }
    };

    let count = payload.events.len();

    for event in &payload.events {
        let captured_at = event.captured_at.unwrap_or_else(chrono::Utc::now);
        let event_type = event.event_type.trim();

        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO recording_events (
                recording_session_id, sequence_number, event_type,
                x, y, button, key_combo, screenshot_object_key, captured_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (recording_session_id, sequence_number) DO UPDATE
            SET event_type = EXCLUDED.event_type,
                x = EXCLUDED.x,
                y = EXCLUDED.y,
                button = EXCLUDED.button,
                key_combo = EXCLUDED.key_combo,
                screenshot_object_key = EXCLUDED.screenshot_object_key,
                captured_at = EXCLUDED.captured_at
            "#,
        )
        .bind(session_id)
        .bind(event.sequence_number)
        .bind(event_type)
        .bind(event.x)
        .bind(event.y)
        .bind(event.button.as_deref())
        .bind(event.key_combo.as_deref())
        .bind(event.screenshot_object_key.as_deref())
        .bind(captured_at)
        .execute(&mut *tx)
        .await
        {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                sequence_number = event.sequence_number,
                "Failed to insert recording event: {}",
                e
            );
            let _ = tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to persist recording events".to_string(),
                }),
            )
                .into_response();
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(
            worker_id = worker.id,
            session_id = session_id,
            "Failed to commit recording events: {}",
            e
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to commit recording events".to_string(),
            }),
        )
            .into_response();
    }

    let stop_requested = session_status != "recording";

    (
        StatusCode::OK,
        Json(PostRecordingEventsResponse {
            status: "success".to_string(),
            count,
            stop_requested,
        }),
    )
        .into_response()
}

#[tracing::instrument(skip(worker, state))]
pub async fn get_recording_screenshot_upload_url_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> impl IntoResponse {
    let session_exists: Option<i64> = match sqlx::query_scalar(
        "SELECT id FROM recording_sessions WHERE id = $1 AND worker_id = $2",
    )
    .bind(session_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                "Error checking recording session for upload URL: {}",
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

    if session_exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Recording session {} not found for this worker", session_id),
            }),
        )
            .into_response();
    }

    let object_key = format!("recordings/{}/event_{}.png", session_id, Uuid::new_v4());
    let storage = state.storage_service();

    match storage
        .generate_presigned_put_url(&object_key, std::time::Duration::from_secs(900))
        .await
    {
        Ok(upload_url) => (
            StatusCode::OK,
            Json(ScreenshotUploadUrlResponse {
                upload_url,
                object_key,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                "Failed to generate presigned upload URL for recording: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to generate upload URL".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(worker, state))]
pub async fn post_worker_stop_recording_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> impl IntoResponse {
    let update_res = sqlx::query(
        r#"
        UPDATE recording_sessions
        SET status = 'completed',
            ended_at = COALESCE(ended_at, now())
        WHERE id = $1 AND worker_id = $2
        RETURNING id
        "#,
    )
    .bind(session_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await;

    match update_res {
        Ok(Some(_)) => {
            tracing::info!(
                worker_id = worker.id,
                session_id = session_id,
                "Recording session finalized by worker"
            );
            (
                StatusCode::OK,
                Json(WorkerStopRecordingResponse {
                    status: "success".to_string(),
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Recording session {} not found for this worker", session_id),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                session_id = session_id,
                "Failed to update recording session status: {}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error stopping recording session".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_task_run_screenshot_commit_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    payload: Option<Json<CommitScreenshotRequest>>,
) -> impl IntoResponse {
    let req = payload.map(|Json(p)| p).unwrap_or(CommitScreenshotRequest {
        step_id: None,
        object_key: None,
        width: None,
        height: None,
    });

    let run_exists: Option<i64> = match sqlx::query_scalar(
        "SELECT id FROM task_runs WHERE id = $1 AND worker_id = $2",
    )
    .bind(task_run_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Error checking task run for screenshot commit: {}",
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

    if run_exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Task run {} not found for this worker", task_run_id),
            }),
        )
            .into_response();
    }

    let object_key = req.object_key.unwrap_or_else(|| format!("runs/{}/step.png", task_run_id));

    if let Some(step_id) = req.step_id {
        let update_res = sqlx::query(
            r#"
            UPDATE task_run_steps
            SET screenshot_object_key = $1
            WHERE task_run_id = $2 AND step_id = $3
            "#,
        )
        .bind(&object_key)
        .bind(task_run_id)
        .bind(step_id)
        .execute(&state.db)
        .await;

        if let Err(e) = update_res {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                step_id = step_id,
                "Failed to update task_run_steps screenshot_object_key: {}",
                e
            );
        }
    } else {
        // Update the most recently recorded step for this task run if step_id not explicitly provided
        let update_res = sqlx::query(
            r#"
            UPDATE task_run_steps
            SET screenshot_object_key = $1
            WHERE id = (
                SELECT id FROM task_run_steps
                WHERE task_run_id = $2
                ORDER BY id DESC
                LIMIT 1
            )
            "#,
        )
        .bind(&object_key)
        .bind(task_run_id)
        .execute(&state.db)
        .await;

        if let Err(e) = update_res {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                "Failed to update latest task_run_steps screenshot_object_key: {}",
                e
            );
        }
    }

    (
        StatusCode::OK,
        Json(CommitScreenshotResponse {
            status: "success".to_string(),
            object_key,
        }),
    )
        .into_response()
}

#[tracing::instrument(skip(worker, state))]
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

#[tracing::instrument(skip(worker, state, payload))]
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

#[tracing::instrument(skip(state, payload))]
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

        let rec_resp = NextAssignmentResponse::StartRecording {
            recording_session_id: 101,
        };
        let rec_json = serde_json::to_string(&rec_resp).expect("Failed to serialize StartRecording");
        assert_eq!(rec_json, r#"{"type":"start_recording","recording_session_id":101}"#);

        let rec_deserialized: NextAssignmentResponse = serde_json::from_str(&rec_json)
            .expect("Failed to deserialize StartRecording");
        assert_eq!(rec_deserialized, rec_resp);

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

    #[test]
    fn test_step_result_validation_and_deserialization() {
        assert!(is_valid_step_result("success"));
        assert!(is_valid_step_result("failed"));
        assert!(is_valid_step_result("branch_matched"));
        assert!(is_valid_step_result("branch_not_matched"));
        assert!(!is_valid_step_result("unknown"));

        let json_data = r#"{
            "step_id": 101,
            "result": "success",
            "captured_rgb": [255, 128, 0],
            "captured_found": true,
            "captured_xy": [100, 200],
            "screenshot_object_key": "runs/1/step_101.png",
            "variable_updates": [
                {
                    "variable_id": 5,
                    "value": {"x": 100, "y": 200}
                }
            ]
        }"#;

        let req: StepResultRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize StepResultRequest");
        assert_eq!(req.step_id, 101);
        assert_eq!(req.result, "success");
        assert_eq!(req.captured_rgb, Some(vec![255, 128, 0]));
        assert_eq!(req.captured_found, Some(true));
        assert_eq!(req.captured_xy, Some(vec![100, 200]));
        assert_eq!(
            req.screenshot_object_key.as_deref(),
            Some("runs/1/step_101.png")
        );
        let updates = req.variable_updates.expect("Expected variable updates");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].variable_id, 5);
    }

    #[test]
    fn test_screenshot_upload_url_response_serialization() {
        let resp = ScreenshotUploadUrlResponse {
            upload_url: "http://localhost:9000/deskdispatch-bucket/runs/1/step_123.png?X-Amz-Signature=abc".to_string(),
            object_key: "runs/1/step_123.png".to_string(),
        };

        let json_str = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(json_str.contains(r#""upload_url":"http://localhost:9000/deskdispatch-bucket/runs/1/step_123.png?X-Amz-Signature=abc""#));
        assert!(json_str.contains(r#""object_key":"runs/1/step_123.png""#));

        let deserialized: ScreenshotUploadUrlResponse = serde_json::from_str(&json_str).expect("Deserialization failed");
        assert_eq!(deserialized, resp);
    }

    #[test]
    fn test_commit_screenshot_request_deserialization() {
        let json_data = r#"{
            "step_id": 101,
            "object_key": "runs/1/step_101.png",
            "width": 1920,
            "height": 1080
        }"#;

        let req: CommitScreenshotRequest = serde_json::from_str(json_data).expect("Deserialization failed");
        assert_eq!(req.step_id, Some(101));
        assert_eq!(req.object_key.as_deref(), Some("runs/1/step_101.png"));
        assert_eq!(req.width, Some(1920));
        assert_eq!(req.height, Some(1080));

        let resp = CommitScreenshotResponse {
            status: "success".to_string(),
            object_key: "runs/1/step_101.png".to_string(),
        };

        let resp_json = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(resp_json.contains(r#""status":"success""#));
        assert!(resp_json.contains(r#""object_key":"runs/1/step_101.png""#));
    }

    #[test]
    fn test_post_recording_events_request_and_response() {
        let json_req = r#"{
            "events": [
                {
                    "sequence_number": 1,
                    "event_type": "mouse_click",
                    "x": 824,
                    "y": 391,
                    "button": "left",
                    "key_combo": null,
                    "screenshot_object_key": "recordings/1/shot1.png"
                },
                {
                    "sequence_number": 2,
                    "event_type": "key_press",
                    "x": null,
                    "y": null,
                    "button": null,
                    "key_combo": "ctrl+v",
                    "screenshot_object_key": null
                }
            ]
        }"#;

        let req: PostRecordingEventsRequest = serde_json::from_str(json_req).expect("Deserialization failed");
        assert_eq!(req.events.len(), 2);
        assert_eq!(req.events[0].sequence_number, 1);
        assert_eq!(req.events[0].event_type, "mouse_click");
        assert_eq!(req.events[0].x, Some(824));
        assert_eq!(req.events[0].y, Some(391));
        assert_eq!(req.events[0].button.as_deref(), Some("left"));

        assert_eq!(req.events[1].sequence_number, 2);
        assert_eq!(req.events[1].event_type, "key_press");
        assert_eq!(req.events[1].key_combo.as_deref(), Some("ctrl+v"));

        let resp = PostRecordingEventsResponse {
            status: "success".to_string(),
            count: 2,
            stop_requested: false,
        };

        let json_resp = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(json_resp.contains(r#""status":"success""#));
        assert!(json_resp.contains(r#""count":2"#));
        assert!(json_resp.contains(r#""stop_requested":false"#));

        let stop_resp = WorkerStopRecordingResponse {
            status: "success".to_string(),
        };
        let json_stop = serde_json::to_string(&stop_resp).expect("Serialization failed");
        assert_eq!(json_stop, r#"{"status":"success"}"#);
    }

    #[test]
    fn test_complete_task_run_request_deserialization() {
        let json_data = r#"{
            "status": "succeeded",
            "error_message": null
        }"#;
        let req: CompleteTaskRunRequest = serde_json::from_str(json_data).expect("Deserialization failed");
        assert_eq!(req.status, "succeeded");
        assert_eq!(req.error_message, None);

        let json_failed = r#"{
            "status": "failed",
            "error_message": "Bitmap not found on target screen"
        }"#;
        let req_failed: CompleteTaskRunRequest = serde_json::from_str(json_failed).expect("Deserialization failed");
        assert_eq!(req_failed.status, "failed");
        assert_eq!(
            req_failed.error_message.as_deref(),
            Some("Bitmap not found on target screen")
        );
    }
}
