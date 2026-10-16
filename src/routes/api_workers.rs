use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
pub use deskdispatch_protocol::*;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use axum::Router;
use axum::routing::{get, post};

use crate::auth::worker::{generate_worker_api_key, hash_token};
use crate::domain::{StepResultStatus, WorkerStatus};
use crate::error::AppError;
use crate::{AppState, auth::AuthWorker};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workers/register",
            post(post_register_worker_handler),
        )
        .route("/api/v1/workers/heartbeat", post(post_heartbeat_handler))
        .route(
            "/api/v1/workers/next-assignment",
            get(get_next_assignment_handler),
        )
        .route("/api/v1/workers/task-runs/{id}", get(get_task_run_handler))
        .route(
            "/api/v1/workers/task-runs/{id}/step-result",
            post(post_step_result_handler),
        )
        .route(
            "/api/v1/workers/task-runs/{id}/complete",
            post(post_complete_task_run_handler),
        )
        .route(
            "/api/v1/workers/task-runs/{id}/screenshot-upload-url",
            get(get_task_run_screenshot_upload_url_handler),
        )
        .route(
            "/api/v1/workers/task-runs/{id}/screenshots/commit",
            post(post_task_run_screenshot_commit_handler),
        )
        .route(
            "/api/v1/workers/task-runs/{id}/screenshots/{*object_key}/commit",
            post(post_task_run_screenshot_commit_with_key_handler),
        )
        .route(
            "/api/v1/workers/recordings/{id}/events",
            post(post_recording_events_handler),
        )
        .route(
            "/api/v1/workers/recordings/{id}/screenshot-upload-url",
            get(get_recording_screenshot_upload_url_handler),
        )
        .route(
            "/api/v1/workers/recordings/{id}/stop",
            post(post_worker_stop_recording_handler),
        )
        .route(
            "/api/v1/workers/bitmaps/upload-url",
            get(get_bitmap_upload_url_handler),
        )
        .route(
            "/api/v1/workers/bitmaps/commit",
            post(post_bitmap_commit_handler),
        )
}

#[tracing::instrument(skip(db))]
pub async fn fetch_full_automation_json(
    db: &PgPool,
    automation_id: i64,
) -> Result<serde_json::Value, sqlx::Error> {
    fetch_full_automation_json_with_overrides(db, automation_id, None).await
}

#[derive(sqlx::FromRow)]
struct ParameterRow {
    name: String,
    param_type: String,
    default_value: String,
}

#[derive(sqlx::FromRow)]
struct VariableRow {
    id: i64,
    name: String,
    var_type: String,
    description: String,
}

#[derive(sqlx::FromRow)]
struct StepRow {
    id: i64,
    label: Option<String>,
    post_delay_ms: i32,
}

#[derive(sqlx::FromRow)]
struct MouseClickRow {
    step_id: i64,
    x: Option<i32>,
    y: Option<i32>,
    x_variable_id: Option<i64>,
    y_variable_id: Option<i64>,
    region_variable_id: Option<i64>,
    button: String,
    click_type: String,
}

#[derive(sqlx::FromRow)]
struct KeyPressRow {
    step_id: i64,
    key_combo: String,
}

#[derive(sqlx::FromRow)]
struct FindPixelRow {
    step_id: i64,
    x: i32,
    y: i32,
    output_variable_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct FindBitmapRow {
    step_id: i64,
    reference_bitmap_id: i64,
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_width: Option<i32>,
    search_height: Option<i32>,
    match_threshold: f32,
    output_found_variable_id: Option<i64>,
    output_x_variable_id: Option<i64>,
    output_y_variable_id: Option<i64>,
    output_region_variable_id: Option<i64>,
    reference_bitmap_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BranchRow {
    step_id: i64,
    condition_type: String,
    x: Option<i32>,
    y: Option<i32>,
    expected_r: Option<i16>,
    expected_g: Option<i16>,
    expected_b: Option<i16>,
    tolerance: Option<i16>,
    reference_bitmap_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_width: Option<i32>,
    search_height: Option<i32>,
    match_threshold: Option<f32>,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
    reference_bitmap_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AutomationMetaRow {
    id: i64,
    name: String,
    description: String,
    status: String,
}

#[tracing::instrument(skip(db))]
pub async fn fetch_full_automation_json_with_overrides(
    db: &PgPool,
    automation_id: i64,
    overrides: Option<&std::collections::HashMap<String, String>>,
) -> Result<serde_json::Value, sqlx::Error> {
    let auto_row = sqlx::query_as::<_, AutomationMetaRow>(
        "SELECT id, name, description, status FROM automations WHERE id = $1",
    )
    .bind(automation_id)
    .fetch_optional(db)
    .await?;

    let (id, name, description, status) = match auto_row {
        Some(auto) => (auto.id, auto.name, auto.description, auto.status),
        None => {
            return Ok(serde_json::json!({
                "id": automation_id,
            }));
        }
    };

    let params_fut = sqlx::query_as::<_, ParameterRow>(
        "SELECT name, param_type, default_value FROM automation_parameters WHERE automation_id = $1 ORDER BY name ASC",
    )
    .bind(automation_id)
    .fetch_all(db);

    let vars_fut = sqlx::query_as::<_, VariableRow>(
        "SELECT id, name, var_type, description FROM automation_variables WHERE automation_id = $1 ORDER BY id ASC",
    )
    .bind(automation_id)
    .fetch_all(db);

    let steps_fut = sqlx::query_as::<_, StepRow>(
        "SELECT id, label, post_delay_ms FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db);

    let mc_fut = sqlx::query_as::<_, MouseClickRow>(
        r#"
        SELECT smc.step_id, smc.x, smc.y, smc.x_variable_id, smc.y_variable_id, smc.region_variable_id, smc.button, smc.click_type
        FROM step_mouse_clicks smc
        JOIN automation_steps s ON smc.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let kp_fut = sqlx::query_as::<_, KeyPressRow>(
        r#"
        SELECT skp.step_id, skp.key_combo
        FROM step_key_presses skp
        JOIN automation_steps s ON skp.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fp_fut = sqlx::query_as::<_, FindPixelRow>(
        r#"
        SELECT sfp.step_id, sfp.x, sfp.y, sfp.output_variable_id
        FROM step_find_pixel_rgb sfp
        JOIN automation_steps s ON sfp.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fb_fut = sqlx::query_as::<_, FindBitmapRow>(
        r#"
        SELECT sfb.step_id, sfb.reference_bitmap_id, sfb.search_x, sfb.search_y, sfb.search_width, sfb.search_height,
               sfb.match_threshold, sfb.output_found_variable_id, sfb.output_x_variable_id, sfb.output_y_variable_id, sfb.output_region_variable_id,
               b.object_storage_key AS reference_bitmap_key
        FROM step_find_bitmap sfb
        JOIN automation_steps s ON sfb.step_id = s.id
        LEFT JOIN bitmaps b ON sfb.reference_bitmap_id = b.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let branch_fut = sqlx::query_as::<_, BranchRow>(
        r#"
        SELECT sb.step_id, sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
               sb.reference_bitmap_id, sb.search_x, sb.search_y, sb.search_width, sb.search_height, sb.match_threshold,
               sb.on_match_step_id, sb.on_no_match_step_id,
               b.object_storage_key AS reference_bitmap_key
        FROM step_branches sb
        JOIN automation_steps s ON sb.step_id = s.id
        LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let (
        param_rows,
        var_rows,
        step_rows,
        mouse_clicks_rows,
        key_presses_rows,
        find_pixel_rows,
        find_bitmap_rows,
        branch_rows,
    ) = tokio::try_join!(
        params_fut, vars_fut, steps_fut, mc_fut, kp_fut, fp_fut, fb_fut, branch_fut
    )?;

    let mut parameters_map = HashMap::with_capacity(param_rows.len());
    for p in param_rows {
        let pval_str = overrides
            .and_then(|o| o.get(&p.name))
            .map(String::as_str)
            .unwrap_or(&p.default_value);

        let val = match p.param_type.as_str() {
            "int" => pval_str
                .parse::<i64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::Value::String(pval_str.to_string())),
            "bool" => {
                let is_true = pval_str.eq_ignore_ascii_case("true")
                    || pval_str == "1"
                    || pval_str.eq_ignore_ascii_case("yes");
                serde_json::Value::Bool(is_true)
            }
            _ => serde_json::Value::String(pval_str.to_string()),
        };
        parameters_map.insert(p.name, val);
    }

    let mut variables_list = Vec::with_capacity(var_rows.len());
    for v in var_rows {
        variables_list.push(AutomationVariablePayload {
            id: v.id,
            name: v.name,
            var_type: v.var_type,
            description: v.description,
        });
    }

    enum StepDetail {
        MouseClick(MouseClickRow),
        KeyPress(String),
        FindPixel(FindPixelRow),
        FindBitmap(FindBitmapRow),
        Branch(BranchRow),
    }

    let mut details_map = HashMap::with_capacity(step_rows.len());
    for r in mouse_clicks_rows {
        details_map.insert(r.step_id, StepDetail::MouseClick(r));
    }
    for r in key_presses_rows {
        details_map.insert(r.step_id, StepDetail::KeyPress(r.key_combo));
    }
    for r in find_pixel_rows {
        details_map.insert(r.step_id, StepDetail::FindPixel(r));
    }
    for r in find_bitmap_rows {
        details_map.insert(r.step_id, StepDetail::FindBitmap(r));
    }
    for r in branch_rows {
        details_map.insert(r.step_id, StepDetail::Branch(r));
    }

    let mut steps_list = Vec::with_capacity(step_rows.len());
    for step in step_rows {
        let step_id = step.id;
        let label = step.label;
        let post_delay_ms = step.post_delay_ms;

        if let Some(detail) = details_map.remove(&step_id) {
            match detail {
                StepDetail::MouseClick(r) => {
                    steps_list.push(StepPayload::MouseClick {
                        id: step_id,
                        label,
                        post_delay_ms,
                        x: r.x,
                        y: r.y,
                        x_variable_id: r.x_variable_id,
                        y_variable_id: r.y_variable_id,
                        region_variable_id: r.region_variable_id,
                        button: r.button,
                        click_type: r.click_type,
                    });
                }
                StepDetail::KeyPress(key_combo) => {
                    steps_list.push(StepPayload::KeyPress {
                        id: step_id,
                        label,
                        post_delay_ms,
                        key_combo,
                    });
                }
                StepDetail::FindPixel(r) => {
                    steps_list.push(StepPayload::FindPixelRgb {
                        id: step_id,
                        label,
                        post_delay_ms,
                        x: r.x,
                        y: r.y,
                        output_variable_id: r.output_variable_id,
                        timeout_ms: 5000,
                        retry_interval_ms: 250,
                    });
                }
                StepDetail::FindBitmap(r) => {
                    steps_list.push(StepPayload::FindBitmap {
                        id: step_id,
                        label,
                        post_delay_ms,
                        reference_bitmap_id: r.reference_bitmap_id,
                        reference_bitmap_key: r.reference_bitmap_key,
                        search_x: r.search_x,
                        search_y: r.search_y,
                        search_width: r.search_width,
                        search_height: r.search_height,
                        match_threshold: r.match_threshold,
                        output_found_variable_id: r.output_found_variable_id,
                        output_x_variable_id: r.output_x_variable_id,
                        output_y_variable_id: r.output_y_variable_id,
                        output_region_variable_id: r.output_region_variable_id,
                        timeout_ms: 5000,
                        retry_interval_ms: 250,
                    });
                }
                StepDetail::Branch(r) => {
                    let condition = if r.condition_type == "pixel_rgb" {
                        let exp_r = r.expected_r.unwrap_or(0);
                        let exp_g = r.expected_g.unwrap_or(0);
                        let exp_b = r.expected_b.unwrap_or(0);
                        BranchConditionPayload::PixelRgb {
                            x: r.x,
                            y: r.y,
                            expected_r: exp_r,
                            expected_g: exp_g,
                            expected_b: exp_b,
                            expected_rgb: [exp_r, exp_g, exp_b],
                            tolerance: r.tolerance.unwrap_or(0),
                            timeout_ms: 5000,
                            retry_interval_ms: 250,
                        }
                    } else {
                        BranchConditionPayload::Bitmap {
                            reference_bitmap_id: r.reference_bitmap_id,
                            reference_bitmap_key: r.reference_bitmap_key,
                            search_x: r.search_x,
                            search_y: r.search_y,
                            search_width: r.search_width,
                            search_height: r.search_height,
                            match_threshold: r.match_threshold,
                            timeout_ms: 5000,
                            retry_interval_ms: 250,
                        }
                    };

                    steps_list.push(StepPayload::Branch {
                        id: step_id,
                        label,
                        post_delay_ms,
                        condition,
                        on_match_step_id: r.on_match_step_id,
                        on_no_match_step_id: r.on_no_match_step_id,
                        timeout_ms: 5000,
                        retry_interval_ms: 250,
                    });
                }
            }
        }
    }

    let payload = FullAutomationPayload {
        id,
        name,
        description,
        status,
        parameters: parameters_map,
        variables: variables_list,
        steps: steps_list,
    };

    serde_json::to_value(payload).map_err(|e| {
        sqlx::Error::Protocol(format!("Failed to serialize automation payload: {}", e))
    })
}

#[tracing::instrument(skip(worker, state))]
pub async fn get_next_assignment_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    if let Err(e) =
        sqlx::query("UPDATE task_worker_pcs SET last_heartbeat_at = now() WHERE id = $1")
            .bind(worker.id)
            .execute(&state.db)
            .await
    {
        tracing::error!(
            worker_id = worker.id,
            "Failed to update worker heartbeat in next-assignment: {}",
            e
        );
    }

    let start_time = std::time::Instant::now();
    let timeout_duration =
        std::time::Duration::from_secs(state.config.worker_long_poll_timeout_secs);
    let mut rx_notify = state.task_queue_notifier.subscribe();

    loop {
        let mut tx = state.db.begin().await?;

        let active_recording: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM recording_sessions WHERE worker_id = $1 AND status = 'recording' ORDER BY id ASC LIMIT 1",
        )
        .bind(worker.id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(recording_session_id) = active_recording {
            tx.commit().await?;
            tracing::info!(
                worker_id = worker.id,
                recording_session_id = recording_session_id,
                "Dispatched start_recording instruction to worker"
            );
            return Ok(Json(NextAssignmentResponse::StartRecording {
                recording_session_id,
            }));
        }

        let select_res = sqlx::query(
            r#"
            SELECT tr.id AS task_run_id, tr.automation_id, tr.dispatched_automation_json, tr.parameter_overrides
            FROM task_runs tr
            LEFT JOIN schedules s ON tr.schedule_id = s.id
            WHERE tr.status = 'queued'
              AND (
                COALESCE(tr.target_worker_group_id, s.worker_group_id) IS NULL
                OR EXISTS (
                  SELECT 1 FROM worker_group_members wgm
                  WHERE wgm.worker_id = $1 AND wgm.group_id = COALESCE(tr.target_worker_group_id, s.worker_group_id)
                )
              )
            ORDER BY tr.queued_at ASC, tr.id ASC
            LIMIT 1
            FOR UPDATE OF tr SKIP LOCKED
            "#,
        )
        .bind(worker.id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = select_res {
            let task_run_id: i64 = row.get("task_run_id");
            let automation_id: i64 = row.get("automation_id");
            let existing_dispatched: Option<serde_json::Value> =
                row.get("dispatched_automation_json");
            let parameter_overrides: Option<serde_json::Value> = row.get("parameter_overrides");

            let automation_json = match existing_dispatched {
                Some(json) => json,
                None => {
                    let mut overrides_map = std::collections::HashMap::new();
                    if let Some(ref po_json) = parameter_overrides {
                        let params_obj = po_json
                            .get("parameters")
                            .and_then(|p| p.as_object())
                            .or_else(|| po_json.as_object());
                        if let Some(obj) = params_obj {
                            for (k, v) in obj {
                                if let Some(v_str) = v.as_str() {
                                    overrides_map.insert(k.clone(), v_str.to_string());
                                } else if !v.is_null() {
                                    overrides_map.insert(k.clone(), v.to_string());
                                }
                            }
                        }
                    }
                    let overrides_ref = if overrides_map.is_empty() {
                        None
                    } else {
                        Some(&overrides_map)
                    };
                    fetch_full_automation_json_with_overrides(
                        &state.db,
                        automation_id,
                        overrides_ref,
                    )
                    .await?
                }
            };

            sqlx::query(
                r#"
                UPDATE task_runs
                SET status = 'running',
                    worker_id = $1,
                    started_at = now(),
                    dispatched_automation_json = $2
                WHERE id = $3
                "#,
            )
            .bind(worker.id)
            .bind(&automation_json)
            .bind(task_run_id)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;

            tracing::info!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                automation_id = automation_id,
                "Claimed queued task run for worker"
            );

            return Ok(Json(NextAssignmentResponse::ExecuteAutomation {
                task_run_id,
                automation: automation_json,
            }));
        } else {
            tx.commit().await?;
            let elapsed = start_time.elapsed();
            if elapsed >= timeout_duration {
                return Ok(Json(NextAssignmentResponse::None));
            }

            let remaining = timeout_duration - elapsed;
            tokio::select! {
                _ = rx_notify.recv() => {},
                _ = tokio::time::sleep(remaining) => {},
            }
        }
    }
}

pub fn is_valid_step_result(result: &str) -> bool {
    result.parse::<StepResultStatus>().is_ok()
}

#[derive(sqlx::FromRow)]
struct StepResultValidationRow {
    automation_id: i64,
    step_count: i64,
    step_valid: bool,
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_step_result_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    Json(payload): Json<StepResultRequest>,
) -> Result<impl IntoResponse, AppError> {
    let result_status: StepResultStatus = payload.result.trim().parse().map_err(|_| {
        AppError::BadRequest(format!(
            "Invalid step result '{}'. Expected one of: success, failed, branch_matched, branch_not_matched",
            payload.result
        ))
    })?;

    let mut tx = state.db.begin().await?;

    // Consolidated validation query: validates run ownership/status, checks step count limit,
    // and asserts step membership in automation in a single database round-trip.
    let validation: Option<StepResultValidationRow> = sqlx::query_as(
        r#"
        SELECT tr.automation_id,
               (SELECT COUNT(*) FROM task_run_steps WHERE task_run_id = tr.id) AS step_count,
               EXISTS(SELECT 1 FROM automation_steps WHERE id = $3 AND automation_id = tr.automation_id) AS step_valid
        FROM task_runs tr
        WHERE tr.id = $1 AND tr.worker_id = $2 AND tr.status IN ('running', 'cancelling')
        "#,
    )
    .bind(task_run_id)
    .bind(worker.id)
    .bind(payload.step_id)
    .fetch_optional(&mut *tx)
    .await?;

    let val = match validation {
        Some(v) => v,
        None => {
            return Err(AppError::NotFound(format!(
                "Running task run {} not found for this worker",
                task_run_id
            )));
        }
    };

    if val.step_count >= 1000 {
        sqlx::query(
            "UPDATE task_runs SET status = 'failed', completed_at = now(), error_message = $1 WHERE id = $2",
        )
        .bind("Exceeded maximum allowed step count limit (1000 steps)")
        .bind(task_run_id)
        .execute(&mut *tx)
        .await?;

        if let Some(ref key) = payload.screenshot_object_key
            && let Err(e) = sqlx::query(
                r#"
            INSERT INTO step_screenshots (step_id, object_storage_key, width, height)
            VALUES ($1, $2, 1920, 1080)
            ON CONFLICT (step_id) DO UPDATE SET object_storage_key = EXCLUDED.object_storage_key
            "#,
            )
            .bind(payload.step_id)
            .bind(key)
            .execute(&mut *tx)
            .await
        {
            tracing::error!(
                task_run_id = task_run_id,
                step_id = payload.step_id,
                "Failed to upsert step_screenshots on step result: {}",
                e
            );
        }

        tx.commit().await?;
        tracing::warn!(
            worker_id = worker.id,
            task_run_id = task_run_id,
            step_count = val.step_count,
            "Failing task run due to exceeding max step count limit (1000 steps)"
        );

        return Err(AppError::BadRequest(
            "Exceeded maximum allowed step count limit (1000 steps)".to_string(),
        ));
    }

    if !val.step_valid {
        return Err(AppError::BadRequest(format!(
            "Step {} does not belong to automation {}",
            payload.step_id, val.automation_id
        )));
    }

    // Batch validate variable membership in a single ANY($1) query
    if let Some(ref updates) = payload.variable_updates
        && !updates.is_empty()
    {
        let var_ids: Vec<i64> = updates.iter().map(|u| u.variable_id).collect();
        let valid_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT id) FROM automation_variables WHERE id = ANY($1) AND automation_id = $2",
        )
        .bind(&var_ids)
        .bind(val.automation_id)
        .fetch_one(&mut *tx)
        .await?;

        if valid_count < var_ids.iter().collect::<std::collections::HashSet<_>>().len() as i64 {
            for item in updates {
                let is_valid: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM automation_variables WHERE id = $1 AND automation_id = $2)",
                )
                .bind(item.variable_id)
                .bind(val.automation_id)
                .fetch_one(&mut *tx)
                .await?;

                if !is_valid {
                    return Err(AppError::BadRequest(format!(
                        "Variable {} does not belong to automation {}",
                        item.variable_id, val.automation_id
                    )));
                }
            }
        }
    }

    let captured_r = payload.captured_r.or_else(|| {
        payload
            .captured_rgb
            .as_ref()
            .and_then(|v| v.first().copied())
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
            .and_then(|v| v.first().copied())
    });
    let captured_y = payload
        .captured_y
        .or_else(|| payload.captured_xy.as_ref().and_then(|v| v.get(1).copied()));

    let completed_at = payload
        .completed_at
        .or(payload.timestamp)
        .unwrap_or_else(chrono::Utc::now);
    let started_at = payload.started_at.unwrap_or(completed_at);

    let step_result_opt: Option<i64> = sqlx::query_scalar(
        r#"
        INSERT INTO task_run_steps (
            task_run_id, seq, step_id, started_at, completed_at, result,
            captured_r, captured_g, captured_b, captured_found, captured_x, captured_y,
            screenshot_object_key
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (task_run_id, seq) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(task_run_id)
    .bind(payload.seq)
    .bind(payload.step_id)
    .bind(started_at)
    .bind(completed_at)
    .bind(result_status.as_str())
    .bind(captured_r)
    .bind(captured_g)
    .bind(captured_b)
    .bind(payload.captured_found)
    .bind(captured_x)
    .bind(captured_y)
    .bind(payload.screenshot_object_key.as_deref())
    .fetch_optional(&mut *tx)
    .await?;

    let step_result_id = match step_result_opt {
        Some(id) => id,
        None => {
            if let Some(seq_num) = payload.seq {
                let existing_id: Option<i64> = sqlx::query_scalar(
                    "SELECT id FROM task_run_steps WHERE task_run_id = $1 AND seq = $2",
                )
                .bind(task_run_id)
                .bind(seq_num)
                .fetch_optional(&mut *tx)
                .await?;

                if let Some(id) = existing_id {
                    tx.commit().await?;
                    return Ok(Json(StepResultResponse {
                        status: "success".to_string(),
                        step_result_id: id,
                    }));
                }
            }

            return Err(AppError::Internal(
                "Failed to record step result".to_string(),
            ));
        }
    };

    sqlx::query("UPDATE task_runs SET current_step_id = $1 WHERE id = $2")
        .bind(payload.step_id)
        .bind(task_run_id)
        .execute(&mut *tx)
        .await?;

    if let Some(updates) = payload.variable_updates
        && !updates.is_empty()
    {
        let var_ids: Vec<i64> = updates.iter().map(|u| u.variable_id).collect();
        let var_vals: Vec<std::borrow::Cow<'_, str>> = updates
            .iter()
            .map(|u| match &u.value {
                serde_json::Value::String(s) => std::borrow::Cow::Borrowed(s.as_str()),
                v => std::borrow::Cow::Owned(v.to_string()),
            })
            .collect();

        sqlx::query(
                r#"
                INSERT INTO task_run_variable_values (task_run_id, variable_id, value, set_at_step_id, set_at)
                SELECT $1, u.variable_id, u.value, $2, now()
                FROM UNNEST($3::int8[], $4::text[]) AS u(variable_id, value)
                ON CONFLICT (task_run_id, variable_id) DO UPDATE
                SET value = EXCLUDED.value,
                    set_at_step_id = EXCLUDED.set_at_step_id,
                    set_at = EXCLUDED.set_at
                "#,
            )
            .bind(task_run_id)
            .bind(payload.step_id)
            .bind(&var_ids)
            .bind(&var_vals)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(Json(StepResultResponse {
        status: "success".to_string(),
        step_result_id,
    }))
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_complete_task_run_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    Json(payload): Json<CompleteTaskRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let final_status = payload.status.trim();
    if final_status != "succeeded" && final_status != "failed" {
        return Err(AppError::BadRequest(format!(
            "Invalid completion status '{}'. Expected 'succeeded' or 'failed'",
            payload.status
        )));
    }

    let update_res = sqlx::query(
        r#"
        UPDATE task_runs
        SET status = CASE WHEN status = 'cancelling' THEN 'cancelled' ELSE $1 END,
            completed_at = now(),
            error_message = COALESCE($2, error_message)
        WHERE id = $3 AND worker_id = $4 AND status IN ('running', 'cancelling')
        RETURNING id
        "#,
    )
    .bind(final_status)
    .bind(payload.error_message.as_deref())
    .bind(task_run_id)
    .bind(worker.id)
    .fetch_optional(&state.db)
    .await?;

    match update_res {
        Some(_) => {
            tracing::info!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                status = final_status,
                "Task run completed"
            );
            Ok(Json(CompleteTaskRunResponse {
                status: "success".to_string(),
            }))
        }
        None => Err(AppError::NotFound(format!(
            "Running task run {} not found for this worker",
            task_run_id
        ))),
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
        WHERE status IN ('running', 'cancelling')
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
        tracing::info!(
            count = count,
            "Swept stalled task runs with silent heartbeats to lost status"
        );
    }

    let timed_out_result = sqlx::query(
        r#"
        UPDATE task_runs
        SET status = 'failed',
            completed_at = now(),
            error_message = COALESCE(error_message, 'Task run exceeded maximum execution timeout (3600 seconds)')
        WHERE status IN ('running', 'cancelling')
          AND started_at IS NOT NULL
          AND started_at < now() - INTERVAL '3600 seconds'
        "#,
    )
    .execute(pool)
    .await?;

    let timeout_count = timed_out_result.rows_affected();
    if timeout_count > 0 {
        tracing::info!(
            count = timeout_count,
            "Swept timed out task runs (>3600s) to failed status"
        );
    }

    let rec_result = sqlx::query(
        r#"
        UPDATE recording_sessions
        SET status = 'discarded'
        WHERE status = 'recording'
          AND EXISTS (
            SELECT 1 FROM task_worker_pcs w
            WHERE w.id = recording_sessions.worker_id
              AND (w.last_heartbeat_at IS NULL OR w.last_heartbeat_at < now() - INTERVAL '90 seconds')
          )
        "#,
    )
    .execute(pool)
    .await?;

    let rec_count = rec_result.rows_affected();
    if rec_count > 0 {
        tracing::info!(
            count = rec_count,
            "Swept stuck recording sessions with silent worker heartbeats to discarded status"
        );
    }

    Ok(count + rec_count)
}

#[tracing::instrument(skip(worker, state))]
pub async fn get_task_run_screenshot_upload_url_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let run_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM task_runs WHERE id = $1 AND worker_id = $2")
            .bind(task_run_id)
            .bind(worker.id)
            .fetch_optional(&state.db)
            .await?;

    if run_exists.is_none() {
        return Err(AppError::NotFound(format!(
            "Running task run {} not found for this worker",
            task_run_id
        )));
    }

    let object_key = format!("runs/{}/step_{}.png", task_run_id, Uuid::new_v4());
    let storage = state.storage_service();

    let upload_url = storage
        .generate_presigned_put_url(&object_key, std::time::Duration::from_secs(900))
        .await
        .map_err(|e| AppError::Internal(format!("Failed to generate upload URL: {}", e)))?;

    Ok(Json(ScreenshotUploadUrlResponse {
        upload_url,
        object_key,
    }))
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_recording_events_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(payload): Json<PostRecordingEventsRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await?;

    let session_row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM recording_sessions WHERE id = $1 AND worker_id = $2")
            .bind(session_id)
            .bind(worker.id)
            .fetch_optional(&mut *tx)
            .await?;

    let session_status = match session_row {
        Some((status,)) => status,
        None => {
            return Err(AppError::NotFound(format!(
                "Recording session {} not found for this worker",
                session_id
            )));
        }
    };

    let count = payload.events.len();

    for event in &payload.events {
        let captured_at = event.captured_at.unwrap_or_else(chrono::Utc::now);
        let event_type = event.event_type.trim();

        sqlx::query(
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
        .await?;
    }

    tx.commit().await?;

    let stop_requested = session_status != "recording";

    Ok(Json(PostRecordingEventsResponse {
        status: "success".to_string(),
        count,
        stop_requested,
    }))
}

#[tracing::instrument(skip(worker, state))]
pub async fn get_recording_screenshot_upload_url_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let session_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM recording_sessions WHERE id = $1 AND worker_id = $2")
            .bind(session_id)
            .bind(worker.id)
            .fetch_optional(&state.db)
            .await?;

    if session_exists.is_none() {
        return Err(AppError::NotFound(format!(
            "Recording session {} not found for this worker",
            session_id
        )));
    }

    let object_key = format!("recordings/{}/event_{}.png", session_id, Uuid::new_v4());
    let storage = state.storage_service();

    let upload_url = storage
        .generate_presigned_put_url(&object_key, std::time::Duration::from_secs(900))
        .await
        .map_err(|e| AppError::Internal(format!("Failed to generate upload URL: {}", e)))?;

    Ok(Json(ScreenshotUploadUrlResponse {
        upload_url,
        object_key,
    }))
}

#[tracing::instrument(skip(worker, state))]
pub async fn post_worker_stop_recording_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
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
    .await?;

    match update_res {
        Some(_) => {
            tracing::info!(
                worker_id = worker.id,
                session_id = session_id,
                "Recording session finalized by worker"
            );
            Ok(Json(WorkerStopRecordingResponse {
                status: "success".to_string(),
            }))
        }
        None => Err(AppError::NotFound(format!(
            "Recording session {} not found for this worker",
            session_id
        ))),
    }
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_task_run_screenshot_commit_with_key_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path((task_run_id, object_key)): Path<(i64, String)>,
    payload: Option<Json<CommitScreenshotRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let mut req = payload.map(|Json(p)| p).unwrap_or(CommitScreenshotRequest {
        step_id: None,
        object_key: None,
        width: None,
        height: None,
    });
    if req.object_key.is_none() {
        req.object_key = Some(object_key);
    }
    post_task_run_screenshot_commit_handler(
        worker,
        State(state),
        Path(task_run_id),
        Some(Json(req)),
    )
    .await
}

#[tracing::instrument(skip(_worker, state))]
pub async fn get_bitmap_upload_url_handler(
    _worker: AuthWorker,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let object_key = format!("bitmaps/worker_{}.png", Uuid::new_v4());
    let storage = state.storage_service();

    let upload_url = storage
        .generate_presigned_put_url(&object_key, std::time::Duration::from_secs(900))
        .await
        .map_err(|e| AppError::Internal(format!("Failed to generate upload URL: {}", e)))?;

    Ok(Json(ScreenshotUploadUrlResponse {
        upload_url,
        object_key,
    }))
}

#[tracing::instrument(skip(_worker, state, req))]
pub async fn post_bitmap_commit_handler(
    _worker: AuthWorker,
    State(state): State<AppState>,
    Json(req): Json<CommitBitmapRequest>,
) -> Result<impl IntoResponse, AppError> {
    let bitmap_name = req.name.trim();
    if bitmap_name.is_empty() {
        return Err(AppError::BadRequest("Bitmap name is required".to_string()));
    }

    let width = req.width.unwrap_or(100);
    let height = req.height.unwrap_or(100);

    let row = sqlx::query(
        r#"
        INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(req.automation_id)
    .bind(bitmap_name)
    .bind(&req.object_key)
    .bind(width)
    .bind(height)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to record bitmap: {}", e)))?;

    let bitmap_id: i64 = row.get("id");

    Ok(Json(CommitBitmapResponse {
        status: "success".to_string(),
        bitmap_id,
        object_key: req.object_key,
    }))
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_task_run_screenshot_commit_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
    payload: Option<Json<CommitScreenshotRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let req = payload.map(|Json(p)| p).unwrap_or(CommitScreenshotRequest {
        step_id: None,
        object_key: None,
        width: None,
        height: None,
    });

    let run_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM task_runs WHERE id = $1 AND worker_id = $2")
            .bind(task_run_id)
            .bind(worker.id)
            .fetch_optional(&state.db)
            .await?;

    if run_exists.is_none() {
        return Err(AppError::NotFound(format!(
            "Task run {} not found for this worker",
            task_run_id
        )));
    }

    let object_key = req
        .object_key
        .unwrap_or_else(|| format!("runs/{}/step.png", task_run_id));

    let width = req.width.unwrap_or(1920);
    let height = req.height.unwrap_or(1080);

    let target_step_id = if let Some(step_id) = req.step_id {
        if let Err(e) = sqlx::query(
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
        .await
        {
            tracing::error!(
                worker_id = worker.id,
                task_run_id = task_run_id,
                step_id = step_id,
                "Failed to update task_run_steps screenshot_object_key: {}",
                e
            );
        }
        Some(step_id)
    } else {
        let step_id_opt: Option<i64> = sqlx::query_scalar(
            r#"
            UPDATE task_run_steps
            SET screenshot_object_key = $1
            WHERE id = (
                SELECT id FROM task_run_steps
                WHERE task_run_id = $2
                ORDER BY id DESC
                LIMIT 1
            )
            RETURNING step_id
            "#,
        )
        .bind(&object_key)
        .bind(task_run_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        step_id_opt
    };

    if let Some(step_id) = target_step_id
        && let Err(e) = sqlx::query(
            r#"
            INSERT INTO step_screenshots (step_id, object_storage_key, width, height)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (step_id) DO UPDATE SET object_storage_key = EXCLUDED.object_storage_key
            "#,
        )
        .bind(step_id)
        .bind(&object_key)
        .bind(width)
        .bind(height)
        .execute(&state.db)
        .await
    {
        tracing::error!(
            worker_id = worker.id,
            task_run_id = task_run_id,
            step_id = step_id,
            "Failed to upsert step_screenshots on commit: {}",
            e
        );
    }

    Ok(Json(CommitScreenshotResponse {
        status: "success".to_string(),
        object_key,
    }))
}

#[tracing::instrument(skip(_worker, state))]
pub async fn get_task_run_handler(
    _worker: AuthWorker,
    State(state): State<AppState>,
    Path(task_run_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await?;

    let row = sqlx::query(
        "SELECT id, automation_id, status, dispatched_automation_json FROM task_runs WHERE id = $1",
    )
    .bind(task_run_id)
    .fetch_optional(&mut *conn)
    .await?;

    match row {
        Some(row) => {
            let id: i64 = row.get("id");
            let automation_id: i64 = row.get("automation_id");
            let status: String = row.get("status");
            let dispatched_json: Option<serde_json::Value> = row.get("dispatched_automation_json");

            let automation_json = match dispatched_json {
                Some(json) => json,
                None => fetch_full_automation_json(&state.db, automation_id).await?,
            };

            Ok(Json(TaskRunResponse {
                task_run_id: id,
                status,
                automation: automation_json,
            }))
        }
        None => Err(AppError::NotFound(format!(
            "Task run {} not found",
            task_run_id
        ))),
    }
}

pub fn is_valid_worker_status(status: &str) -> bool {
    status.parse::<WorkerStatus>().is_ok()
}

pub fn is_agent_version_outdated(agent_version: Option<&str>, min_version: Option<&str>) -> bool {
    let min_ver = match min_version {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return false,
    };

    let agent_ver = match agent_version {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return true,
    };

    let agent_it = agent_ver.split('.').map(|part| {
        part.bytes()
            .take_while(|b| b.is_ascii_digit())
            .fold(0u64, |acc, b| acc * 10 + (b - b'0') as u64)
    });
    let min_it = min_ver.split('.').map(|part| {
        part.bytes()
            .take_while(|b| b.is_ascii_digit())
            .fold(0u64, |acc, b| acc * 10 + (b - b'0') as u64)
    });

    agent_it.cmp(min_it).is_lt()
}

#[tracing::instrument(skip(worker, state, payload))]
pub async fn post_heartbeat_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Json(payload): Json<HeartbeatRequest>,
) -> Result<impl IntoResponse, AppError> {
    if is_agent_version_outdated(
        payload.agent_version.as_deref(),
        state.config.min_agent_version.as_deref(),
    ) {
        let min_ver = state.config.min_agent_version.as_deref().unwrap_or("1.0.0");
        let provided = payload.agent_version.as_deref().unwrap_or("none");
        return Err(AppError::UpgradeRequired(format!(
            "Outdated agent version '{}'. Minimum required agent version is '{}'",
            provided, min_ver
        )));
    }

    let new_status = if let Some(ref status_str) = payload.status {
        let status: WorkerStatus = status_str.trim().parse().map_err(|_| {
            AppError::BadRequest(format!(
                "Invalid status '{}'. Expected one of: offline, online, busy, error",
                status_str
            ))
        })?;
        status.to_string()
    } else {
        if is_valid_worker_status(&worker.status) {
            worker.status.clone()
        } else {
            WorkerStatus::Online.to_string()
        }
    };

    sqlx::query(
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
    .await?;

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
                if status == "cancelling" || status == "cancelled" {
                    cancel_requested = true;
                }
            }
            Ok(None) => {
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

    Ok(Json(HeartbeatResponse {
        status: "success".to_string(),
        cancel_requested,
        poll_interval_secs: state.config.worker_poll_interval_secs,
        heartbeat_interval_secs: state.config.worker_heartbeat_interval_secs,
    }))
}

#[tracing::instrument(skip(state, payload))]
pub async fn post_register_worker_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterWorkerRequest>,
) -> Result<impl IntoResponse, AppError> {
    use secrecy::ExposeSecret;
    use std::sync::atomic::Ordering;

    let psk = payload
        .psk
        .or(payload.pre_shared_key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(provided_psk) = psk {
        if !state.wps_enabled.load(Ordering::Relaxed) {
            return Err(AppError::Unauthorized(
                "Unattended WPS registration is currently disabled".to_string(),
            ));
        }

        let expected_psk = state.config.unattended_registration_psk.expose_secret();
        if provided_psk != expected_psk.as_str() {
            return Err(AppError::Unauthorized("Invalid pre-shared key".to_string()));
        }

        let hostname = payload
            .hostname
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "worker-unattended".to_string());
        let display_name = payload
            .display_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| hostname.clone());
        let mac_address = payload
            .mac_address
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let api_key = generate_worker_api_key();
        let api_key_hash = hash_token(&api_key);

        let row = sqlx::query(
            r#"
            INSERT INTO task_worker_pcs (hostname, display_name, mac_address, api_key_hash, status)
            VALUES ($1, $2, $3, $4, 'offline')
            RETURNING id, hostname, display_name
            "#,
        )
        .bind(&hostname)
        .bind(&display_name)
        .bind(mac_address)
        .bind(&api_key_hash)
        .fetch_one(&state.db)
        .await?;

        let worker_id: i64 = row.get("id");
        let hostname: String = row.get("hostname");
        let display_name: String = row.get("display_name");

        tracing::info!(
            worker_id = worker_id,
            hostname = %hostname,
            "Unattended worker PC registered successfully using WPS pre-shared key"
        );

        return Ok(Json(RegisterWorkerResponse {
            status: "success".to_string(),
            worker_id,
            hostname,
            display_name,
            api_key,
        }));
    }

    let token = payload
        .registration_token
        .or(payload.token)
        .map(|t| t.trim().to_string())
        .unwrap_or_default();

    if token.is_empty() {
        return Err(AppError::BadRequest(
            "registration_token or psk is required".to_string(),
        ));
    }

    let token_hash = hash_token(&token);
    let api_key = generate_worker_api_key();
    let api_key_hash = hash_token(&api_key);

    let row_result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET api_key_hash = $1,
            registration_token_hash = NULL,
            registration_token_expires_at = NULL
        WHERE registration_token_hash = $2
          AND registration_token_expires_at > now()
        RETURNING id, hostname, display_name
        "#,
    )
    .bind(&api_key_hash)
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await?;

    match row_result {
        Some(row) => {
            let worker_id: i64 = row.get("id");
            let hostname: String = row.get("hostname");
            let display_name: String = row.get("display_name");

            tracing::info!(
                worker_id = worker_id,
                hostname = %hostname,
                "Task worker PC registered successfully"
            );

            Ok(Json(RegisterWorkerResponse {
                status: "success".to_string(),
                worker_id,
                hostname,
                display_name,
                api_key,
            }))
        }
        None => Err(AppError::Unauthorized(
            "Invalid or expired registration token".to_string(),
        )),
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
        assert_eq!(req.psk, None);

        let json_data_alt = r#"{"token": "token-456"}"#;
        let req_alt: RegisterWorkerRequest =
            serde_json::from_str(json_data_alt).expect("Failed to deserialize token");
        assert_eq!(req_alt.registration_token, None);
        assert_eq!(req_alt.token.as_deref(), Some("token-456"));

        let json_psk = r#"{
            "psk": "deskdispatch_wps_psk_secret",
            "hostname": "unattended-node-01",
            "display_name": "Unattended Worker 01",
            "mac_address": "00:11:22:33:44:55"
        }"#;
        let req_psk: RegisterWorkerRequest =
            serde_json::from_str(json_psk).expect("Failed to deserialize psk registration");
        assert_eq!(req_psk.psk.as_deref(), Some("deskdispatch_wps_psk_secret"));
        assert_eq!(req_psk.hostname.as_deref(), Some("unattended-node-01"));
        assert_eq!(
            req_psk.display_name.as_deref(),
            Some("Unattended Worker 01")
        );
        assert_eq!(req_psk.mac_address.as_deref(), Some("00:11:22:33:44:55"));
    }

    #[test]
    fn test_api_key_generation_and_sha256_hash() {
        let api_key = generate_worker_api_key();
        assert!(api_key.starts_with("dd_pk_"));
        assert_eq!(api_key.len(), 70);

        let hash = hash_token(&api_key);
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
    fn test_is_agent_version_outdated() {
        assert!(!is_agent_version_outdated(Some("1.0.0"), Some("1.0.0")));
        assert!(!is_agent_version_outdated(Some("1.1.0"), Some("1.0.0")));
        assert!(!is_agent_version_outdated(Some("2.0.0"), Some("1.2.3")));

        assert!(is_agent_version_outdated(Some("0.9.9"), Some("1.0.0")));
        assert!(is_agent_version_outdated(Some("1.0.0"), Some("1.0.1")));
        assert!(is_agent_version_outdated(None, Some("1.0.0")));
        assert!(is_agent_version_outdated(Some(""), Some("1.0.0")));

        assert!(!is_agent_version_outdated(Some("0.1.0"), None));
        assert!(!is_agent_version_outdated(None, None));
    }

    #[test]
    fn test_is_agent_version_outdated_version_comparison_edge_cases() {
        // Double-digit component comparison: 1.10.0 is newer than 1.9.0
        assert!(!is_agent_version_outdated(Some("1.10.0"), Some("1.9.0")));
        assert!(is_agent_version_outdated(Some("1.9.5"), Some("1.10.0")));

        // Whitespace padded version strings should be trimmed and correctly evaluated
        assert!(!is_agent_version_outdated(
            Some("  1.2.0  "),
            Some(" 1.1.0 ")
        ));
        assert!(is_agent_version_outdated(Some(" 1.0.0 "), Some(" 1.2.0 ")));

        // Whitespace-only minimum version is treated as unconstrained (not outdated)
        assert!(!is_agent_version_outdated(Some("1.0.0"), Some("   ")));

        // Whitespace-only agent version is treated as missing agent version (outdated)
        assert!(is_agent_version_outdated(Some("   "), Some("1.0.0")));

        // Pre-release suffixes like -beta or -rc1 should strip suffixes and parse base numeric parts
        assert!(!is_agent_version_outdated(Some("2.0.0-rc1"), Some("1.9.0")));
        assert!(is_agent_version_outdated(Some("1.0.0-beta"), Some("1.1.0")));
    }

    #[test]
    fn test_heartbeat_response_serialization() {
        let resp = HeartbeatResponse {
            status: "success".to_string(),
            cancel_requested: true,
            poll_interval_secs: 5,
            heartbeat_interval_secs: 15,
        };

        let json_str = serde_json::to_string(&resp).expect("Failed to serialize HeartbeatResponse");
        assert!(json_str.contains(r#""status":"success""#));
        assert!(json_str.contains(r#""cancel_requested":true"#));
        assert!(json_str.contains(r#""poll_interval_secs":5"#));
        assert!(json_str.contains(r#""heartbeat_interval_secs":15"#));
    }

    #[test]
    fn test_next_assignment_response_serialization() {
        let resp = NextAssignmentResponse::None;
        let json_str =
            serde_json::to_string(&resp).expect("Failed to serialize NextAssignmentResponse");
        assert_eq!(json_str, r#"{"type":"none"}"#);

        let rec_resp = NextAssignmentResponse::StartRecording {
            recording_session_id: 101,
        };
        let rec_json =
            serde_json::to_string(&rec_resp).expect("Failed to serialize StartRecording");
        assert_eq!(
            rec_json,
            r#"{"type":"start_recording","recording_session_id":101}"#
        );

        let rec_deserialized: NextAssignmentResponse =
            serde_json::from_str(&rec_json).expect("Failed to deserialize StartRecording");
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
        let exec_json =
            serde_json::to_string(&exec_resp).expect("Failed to serialize ExecuteAutomation");
        assert!(exec_json.contains(r#""type":"execute_automation""#));
        assert!(exec_json.contains(r#""task_run_id":4821"#));
        assert!(exec_json.contains(r#""automation":{"id":12,"parameters":{"click_tolerance":8}}"#));

        let exec_deserialized: NextAssignmentResponse =
            serde_json::from_str(&exec_json).expect("Failed to deserialize ExecuteAutomation");
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
            upload_url:
                "http://localhost:9000/deskdispatch-bucket/runs/1/step_123.png?X-Amz-Signature=abc"
                    .to_string(),
            object_key: "runs/1/step_123.png".to_string(),
        };

        let json_str = serde_json::to_string(&resp).expect("Serialization failed");
        assert!(json_str.contains(r#""upload_url":"http://localhost:9000/deskdispatch-bucket/runs/1/step_123.png?X-Amz-Signature=abc""#));
        assert!(json_str.contains(r#""object_key":"runs/1/step_123.png""#));

        let deserialized: ScreenshotUploadUrlResponse =
            serde_json::from_str(&json_str).expect("Deserialization failed");
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

        let req: CommitScreenshotRequest =
            serde_json::from_str(json_data).expect("Deserialization failed");
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

        let req: PostRecordingEventsRequest =
            serde_json::from_str(json_req).expect("Deserialization failed");
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
    fn test_commit_screenshot_with_path_key() {
        let req = CommitScreenshotRequest {
            step_id: Some(10),
            object_key: Some("runs/1/custom_key.png".to_string()),
            width: Some(1920),
            height: Some(1080),
        };
        assert_eq!(req.object_key.as_deref(), Some("runs/1/custom_key.png"));
    }

    #[test]
    fn test_complete_task_run_request_deserialization() {
        let json_data = r#"{
            "status": "succeeded",
            "error_message": null
        }"#;
        let req: CompleteTaskRunRequest =
            serde_json::from_str(json_data).expect("Deserialization failed");
        assert_eq!(req.status, "succeeded");
        assert_eq!(req.error_message, None);

        let json_failed = r#"{
            "status": "failed",
            "error_message": "Bitmap not found on target screen"
        }"#;
        let req_failed: CompleteTaskRunRequest =
            serde_json::from_str(json_failed).expect("Deserialization failed");
        assert_eq!(req_failed.status, "failed");
        assert_eq!(
            req_failed.error_message.as_deref(),
            Some("Bitmap not found on target screen")
        );
    }
}
