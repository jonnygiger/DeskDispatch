use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use sqlx::{PgPool, Row};

use super::types::*;
use super::variables::fetch_automation_variables;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, log_audit};
use crate::routes::auth::HtmlTemplate;

pub fn validate_literal_or_reference<T>(
    mode: Option<&str>,
    literal: Option<T>,
    variable_id: Option<i64>,
    field_name: &str,
) -> Result<(Option<T>, Option<i64>), String> {
    let var_id = variable_id.filter(|&v| v > 0);
    let effective_mode = match mode {
        Some("variable") => "variable",
        Some("fixed") => "fixed",
        _ => {
            if literal.is_some() && var_id.is_none() {
                "fixed"
            } else if literal.is_none() && var_id.is_some() {
                "variable"
            } else if literal.is_some() && var_id.is_some() {
                return Err(format!(
                    "Cannot specify both a fixed value and a variable for {}.",
                    field_name
                ));
            } else {
                return Err(format!(
                    "Please specify either a fixed value or a variable for {}.",
                    field_name
                ));
            }
        }
    };

    match effective_mode {
        "variable" => {
            if let Some(vid) = var_id {
                Ok((None, Some(vid)))
            } else {
                Err(format!(
                    "Please select a valid variable for {}.",
                    field_name
                ))
            }
        }
        "fixed" => {
            if let Some(val) = literal {
                Ok((Some(val), None))
            } else {
                Err(format!("Please provide a fixed value for {}.", field_name))
            }
        }
        _ => unreachable!(),
    }
}

pub async fn fetch_automation_step_options(
    db: &PgPool,
    automation_id: i64,
    exclude_step_id: Option<i64>,
) -> Vec<StepOption> {
    let raw_steps = sqlx::query(
        "SELECT id, step_type, label, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    raw_steps
        .into_iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            let id: i64 = row.get("id");
            if exclude_step_id == Some(id) {
                return None;
            }
            let step_type: String = row.get("step_type");
            let label: Option<String> = row.get("label");
            let position: f64 = row.get("position");
            Some(StepOption {
                id,
                step_type,
                label,
                position,
                display_number: idx + 1,
            })
        })
        .collect()
}

pub async fn fetch_available_bitmaps(db: &PgPool, automation_id: i64) -> Vec<BitmapOption> {
    sqlx::query_as::<_, BitmapOption>(
        "SELECT id, name, width, height FROM bitmaps WHERE automation_id = $1 OR automation_id IS NULL ORDER BY name ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

pub fn generate_mouse_click_summary(
    x: Option<i32>,
    y: Option<i32>,
    x_var_name: Option<&str>,
    y_var_name: Option<&str>,
    button: &str,
    click_type: &str,
) -> String {
    let x_str = match x_var_name {
        Some(name) if !name.trim().is_empty() => format!("«{}»", name.trim()),
        _ => match x {
            Some(val) => val.to_string(),
            None => "variable".to_string(),
        },
    };

    let y_str = match y_var_name {
        Some(name) if !name.trim().is_empty() => format!("«{}»", name.trim()),
        _ => match y {
            Some(val) => val.to_string(),
            None => "variable".to_string(),
        },
    };

    format!("Click ({}, {}) [{}, {}]", x_str, y_str, button, click_type)
}

pub fn generate_find_pixel_rgb_summary(x: i32, y: i32, output_var_name: Option<&str>) -> String {
    match output_var_name {
        Some(name) if !name.trim().is_empty() => {
            format!("Read pixel at ({}, {}) → store as «{}»", x, y, name.trim())
        }
        _ => format!("Read pixel at ({}, {})", x, y),
    }
}

pub fn generate_find_bitmap_summary(
    bitmap_name: &str,
    output_found_var_name: Option<&str>,
) -> String {
    match output_found_var_name {
        Some(name) if !name.trim().is_empty() => {
            format!(
                "Search for «{}» on screen → store as «{}»",
                bitmap_name,
                name.trim()
            )
        }
        _ => format!("Search for «{}» on screen", bitmap_name),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn generate_branch_summary(
    condition_type: &str,
    x: Option<i32>,
    y: Option<i32>,
    expected_r: Option<i16>,
    expected_g: Option<i16>,
    expected_b: Option<i16>,
    tolerance: Option<i16>,
    bitmap_name: Option<&str>,
    match_target: Option<&str>,
    no_match_target: Option<&str>,
) -> String {
    let match_str = match_target.unwrap_or("Next Step");
    let no_match_str = no_match_target.unwrap_or("Next Step");

    if condition_type == "pixel_rgb" {
        let px = x.unwrap_or(0);
        let py = y.unwrap_or(0);
        let r = expected_r.unwrap_or(0);
        let g = expected_g.unwrap_or(0);
        let b = expected_b.unwrap_or(0);
        let tol = tolerance.unwrap_or(0);
        format!(
            "BRANCH: if pixel at ({}, {}) ≈ RGB({},{},{}) ±{} → go to {}, else → go to {}",
            px, py, r, g, b, tol, match_str, no_match_str
        )
    } else {
        let bname = bitmap_name.unwrap_or("bitmap");
        format!(
            "BRANCH: if bitmap «{}» found → go to {}, else → go to {}",
            bname, match_str, no_match_str
        )
    }
}

/// Bulk fetch and format all step view items for an automation.
/// Optimization: Bulk-loads steps and all step subtype tables concurrently in O(1) database
/// queries using `tokio::join!`, avoiding N+1 database queries per step on automation views.
#[allow(clippy::type_complexity)]
pub async fn fetch_automation_step_views(db: &PgPool, automation_id: i64) -> Vec<StepViewItem> {
    let raw_steps = match sqlx::query(
        "SELECT id, step_type, label, post_delay_ms, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching steps for automation {}: {}", automation_id, e);
            return Vec::new();
        }
    };

    if raw_steps.is_empty() {
        return Vec::new();
    }

    // Build target step label map in memory (step_id -> "Step N (label/type)")
    let mut step_target_map: std::collections::HashMap<i64, String> =
        std::collections::HashMap::new();
    for (idx, row) in raw_steps.iter().enumerate() {
        let step_id: i64 = row.get("id");
        let step_num = idx + 1;
        let step_type: String = row.get("step_type");
        let label: Option<String> = row.get("label");

        let target_str = match label {
            Some(lbl) if !lbl.trim().is_empty() => format!("Step {} ({})", step_num, lbl.trim()),
            _ => format!("Step {} ({})", step_num, step_type),
        };
        step_target_map.insert(step_id, target_str);
    }

    // Run 5 bulk subtype queries concurrently across PgPool connections using tokio::join!
    let (
        kp_res,
        mc_res,
        fp_res,
        fb_res,
        br_res,
    ) = tokio::join!(
        sqlx::query(
            r#"
            SELECT skp.step_id, skp.key_combo
            FROM step_key_presses skp
            JOIN automation_steps s ON skp.step_id = s.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query(
            r#"
            SELECT mc.step_id, mc.x, mc.y, mc.button, mc.click_type, vx.name AS x_var_name, vy.name AS y_var_name
            FROM step_mouse_clicks mc
            JOIN automation_steps s ON mc.step_id = s.id
            LEFT JOIN automation_variables vx ON mc.x_variable_id = vx.id
            LEFT JOIN automation_variables vy ON mc.y_variable_id = vy.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query(
            r#"
            SELECT fp.step_id, fp.x, fp.y, v.name AS var_name
            FROM step_find_pixel_rgb fp
            JOIN automation_steps s ON fp.step_id = s.id
            LEFT JOIN automation_variables v ON fp.output_variable_id = v.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query(
            r#"
            SELECT fb.step_id, b.name AS bitmap_name, v.name AS var_name
            FROM step_find_bitmap fb
            JOIN automation_steps s ON fb.step_id = s.id
            JOIN bitmaps b ON fb.reference_bitmap_id = b.id
            LEFT JOIN automation_variables v ON fb.output_found_variable_id = v.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query(
            r#"
            SELECT
                sb.step_id, sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
                b.name AS bitmap_name, sb.on_match_step_id, sb.on_no_match_step_id
            FROM step_branches sb
            JOIN automation_steps s ON sb.step_id = s.id
            LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),
    );

    let key_presses_map: std::collections::HashMap<i64, String> = kp_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let step_id: i64 = r.get("step_id");
            let key_combo: String = r.get("key_combo");
            (step_id, key_combo)
        })
        .collect();

    let mouse_clicks_map: std::collections::HashMap<
        i64,
        (
            Option<i32>,
            Option<i32>,
            Option<String>,
            Option<String>,
            String,
            String,
        ),
    > = mc_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let step_id: i64 = r.get("step_id");
            let x: Option<i32> = r.get("x");
            let y: Option<i32> = r.get("y");
            let x_var_name: Option<String> = r.get("x_var_name");
            let y_var_name: Option<String> = r.get("y_var_name");
            let button: String = r.get("button");
            let click_type: String = r.get("click_type");
            (step_id, (x, y, x_var_name, y_var_name, button, click_type))
        })
        .collect();

    let find_pixel_map: std::collections::HashMap<i64, (i32, i32, Option<String>)> = fp_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let step_id: i64 = r.get("step_id");
            let x: i32 = r.get("x");
            let y: i32 = r.get("y");
            let var_name: Option<String> = r.get("var_name");
            (step_id, (x, y, var_name))
        })
        .collect();

    let find_bitmap_map: std::collections::HashMap<i64, (String, Option<String>)> = fb_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let step_id: i64 = r.get("step_id");
            let bitmap_name: String = r.get("bitmap_name");
            let var_name: Option<String> = r.get("var_name");
            (step_id, (bitmap_name, var_name))
        })
        .collect();

    let branch_map: std::collections::HashMap<
        i64,
        (
            String,
            Option<i32>,
            Option<i32>,
            Option<i16>,
            Option<i16>,
            Option<i16>,
            Option<i16>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ),
    > = br_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let step_id: i64 = r.get("step_id");
            let condition_type: String = r.get("condition_type");
            let x: Option<i32> = r.get("x");
            let y: Option<i32> = r.get("y");
            let expected_r: Option<i16> = r.get("expected_r");
            let expected_g: Option<i16> = r.get("expected_g");
            let expected_b: Option<i16> = r.get("expected_b");
            let tolerance: Option<i16> = r.get("tolerance");
            let bitmap_name: Option<String> = r.get("bitmap_name");
            let on_match_step_id: Option<i64> = r.get("on_match_step_id");
            let on_no_match_step_id: Option<i64> = r.get("on_no_match_step_id");
            (
                step_id,
                (
                    condition_type,
                    x,
                    y,
                    expected_r,
                    expected_g,
                    expected_b,
                    tolerance,
                    bitmap_name,
                    on_match_step_id,
                    on_no_match_step_id,
                ),
            )
        })
        .collect();

    let mut steps = Vec::with_capacity(raw_steps.len());
    for (idx, row) in raw_steps.into_iter().enumerate() {
        let step_id: i64 = row.get("id");
        let step_type: String = row.get("step_type");
        let label: Option<String> = row.get("label");
        let post_delay_ms: i32 = row.get("post_delay_ms");
        let position: f64 = row.get("position");

        let description = match step_type.as_str() {
            "key_press" => {
                if let Some(combo) = key_presses_map.get(&step_id) {
                    format!("Press {}", combo)
                } else {
                    "Press key".to_string()
                }
            }
            "mouse_click" => {
                if let Some((x, y, x_var_name, y_var_name, button, click_type)) =
                    mouse_clicks_map.get(&step_id)
                {
                    generate_mouse_click_summary(
                        *x,
                        *y,
                        x_var_name.as_deref(),
                        y_var_name.as_deref(),
                        button,
                        click_type,
                    )
                } else {
                    "Click mouse".to_string()
                }
            }
            "find_pixel_rgb" => {
                if let Some((x, y, var_name)) = find_pixel_map.get(&step_id) {
                    generate_find_pixel_rgb_summary(*x, *y, var_name.as_deref())
                } else {
                    "Read pixel at coordinate".to_string()
                }
            }
            "find_bitmap" => {
                if let Some((bitmap_name, var_name)) = find_bitmap_map.get(&step_id) {
                    generate_find_bitmap_summary(bitmap_name, var_name.as_deref())
                } else {
                    "Search for bitmap on screen".to_string()
                }
            }
            "branch" => {
                if let Some((
                    condition_type,
                    x,
                    y,
                    expected_r,
                    expected_g,
                    expected_b,
                    tolerance,
                    bitmap_name,
                    match_id,
                    no_match_id,
                )) = branch_map.get(&step_id)
                {
                    let match_target_str = match_id
                        .and_then(|m_id| step_target_map.get(&m_id).cloned())
                        .unwrap_or_else(|| "Next Step".to_string());

                    let no_match_target_str = no_match_id
                        .and_then(|n_id| step_target_map.get(&n_id).cloned())
                        .unwrap_or_else(|| "Next Step".to_string());

                    generate_branch_summary(
                        condition_type,
                        *x,
                        *y,
                        *expected_r,
                        *expected_g,
                        *expected_b,
                        *tolerance,
                        bitmap_name.as_deref(),
                        Some(&match_target_str),
                        Some(&no_match_target_str),
                    )
                } else {
                    "BRANCH evaluation".to_string()
                }
            }
            _ => "Unknown step".to_string(),
        };

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

    steps
}

pub async fn fetch_step_description(db: &PgPool, step_id: i64, step_type: &str) -> String {
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
            let row = sqlx::query(
                r#"
                SELECT mc.x, mc.y, mc.button, mc.click_type, vx.name AS x_var_name, vy.name AS y_var_name
                FROM step_mouse_clicks mc
                LEFT JOIN automation_variables vx ON mc.x_variable_id = vx.id
                LEFT JOIN automation_variables vy ON mc.y_variable_id = vy.id
                WHERE mc.step_id = $1
                "#,
            )
            .bind(step_id)
            .fetch_optional(db)
            .await;

            if let Ok(Some(r)) = row {
                let x: Option<i32> = r.get("x");
                let y: Option<i32> = r.get("y");
                let x_var_name: Option<String> = r.get("x_var_name");
                let y_var_name: Option<String> = r.get("y_var_name");
                let button: String = r.get("button");
                let click_type: String = r.get("click_type");
                generate_mouse_click_summary(
                    x,
                    y,
                    x_var_name.as_deref(),
                    y_var_name.as_deref(),
                    &button,
                    &click_type,
                )
            } else {
                "Click mouse".to_string()
            }
        }
        "find_pixel_rgb" => {
            let row = sqlx::query(
                r#"
                SELECT fp.x, fp.y, v.name AS var_name
                FROM step_find_pixel_rgb fp
                LEFT JOIN automation_variables v ON fp.output_variable_id = v.id
                WHERE fp.step_id = $1
                "#,
            )
            .bind(step_id)
            .fetch_optional(db)
            .await;

            if let Ok(Some(r)) = row {
                let x: i32 = r.get("x");
                let y: i32 = r.get("y");
                let var_name: Option<String> = r.get("var_name");
                generate_find_pixel_rgb_summary(x, y, var_name.as_deref())
            } else {
                "Read pixel at coordinate".to_string()
            }
        }
        "find_bitmap" => {
            let row = sqlx::query(
                r#"
                SELECT b.name AS bitmap_name, v.name AS var_name
                FROM step_find_bitmap fb
                JOIN bitmaps b ON fb.reference_bitmap_id = b.id
                LEFT JOIN automation_variables v ON fb.output_found_variable_id = v.id
                WHERE fb.step_id = $1
                "#,
            )
            .bind(step_id)
            .fetch_optional(db)
            .await;

            if let Ok(Some(r)) = row {
                let bitmap_name: String = r.get("bitmap_name");
                let var_name: Option<String> = r.get("var_name");
                generate_find_bitmap_summary(&bitmap_name, var_name.as_deref())
            } else {
                "Search for bitmap on screen".to_string()
            }
        }
        "branch" => {
            let row = sqlx::query(
                r#"
                SELECT
                    sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
                    b.name AS bitmap_name,
                    sm.id AS match_id, sm.step_type AS match_type, sm.label AS match_label, sm.position AS match_pos,
                    sn.id AS no_match_id, sn.step_type AS no_match_type, sn.label AS no_match_label, sn.position AS no_match_pos,
                    s.automation_id
                FROM step_branches sb
                JOIN automation_steps s ON sb.step_id = s.id
                LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
                LEFT JOIN automation_steps sm ON sb.on_match_step_id = sm.id
                LEFT JOIN automation_steps sn ON sb.on_no_match_step_id = sn.id
                WHERE sb.step_id = $1
                "#,
            )
            .bind(step_id)
            .fetch_optional(db)
            .await;

            if let Ok(Some(r)) = row {
                let condition_type: String = r.get("condition_type");
                let x: Option<i32> = r.get("x");
                let y: Option<i32> = r.get("y");
                let expected_r: Option<i16> = r.get("expected_r");
                let expected_g: Option<i16> = r.get("expected_g");
                let expected_b: Option<i16> = r.get("expected_b");
                let tolerance: Option<i16> = r.get("tolerance");
                let bitmap_name: Option<String> = r.get("bitmap_name");
                let automation_id: i64 = r.get("automation_id");

                let match_target_str = if let Some(m_id) = r.get::<Option<i64>, _>("match_id") {
                    get_step_target_label(
                        db,
                        automation_id,
                        m_id,
                        r.get("match_label"),
                        r.get("match_type"),
                    )
                    .await
                } else {
                    "Next Step".to_string()
                };

                let no_match_target_str = if let Some(n_id) = r.get::<Option<i64>, _>("no_match_id")
                {
                    get_step_target_label(
                        db,
                        automation_id,
                        n_id,
                        r.get("no_match_label"),
                        r.get("no_match_type"),
                    )
                    .await
                } else {
                    "Next Step".to_string()
                };

                generate_branch_summary(
                    &condition_type,
                    x,
                    y,
                    expected_r,
                    expected_g,
                    expected_b,
                    tolerance,
                    bitmap_name.as_deref(),
                    Some(&match_target_str),
                    Some(&no_match_target_str),
                )
            } else {
                "BRANCH evaluation".to_string()
            }
        }
        _ => "Unknown step".to_string(),
    }
}

async fn get_step_target_label(
    db: &PgPool,
    automation_id: i64,
    step_id: i64,
    label: Option<String>,
    step_type: Option<String>,
) -> String {
    let step_num = sqlx::query(
        "SELECT COUNT(*) AS num FROM automation_steps WHERE automation_id = $1 AND position <= (SELECT position FROM automation_steps WHERE id = $2)",
    )
    .bind(automation_id)
    .bind(step_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|r| r.get::<i64, _>("num"))
    .unwrap_or(0);

    match label {
        Some(lbl) if !lbl.trim().is_empty() => format!("Step {} ({})", step_num, lbl.trim()),
        _ => format!(
            "Step {} ({})",
            step_num,
            step_type.as_deref().unwrap_or("step")
        ),
    }
}

/// GET /automations/{id}/steps/new
#[tracing::instrument(skip(user))]
pub async fn get_step_type_picker_handler(
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    HtmlTemplate(StepTypePickerTemplate {
        user,
        automation_id: id,
    })
}

/// GET /automations/{id}/steps/new/key_press
#[tracing::instrument(skip(user))]
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

/// GET /automations/{id}/steps/new/branch
#[tracing::instrument(skip(state, user))]
pub async fn get_new_branch_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewBranchQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let condition_type = query
        .condition_type
        .unwrap_or_else(|| "pixel_rgb".to_string());
    let steps = fetch_automation_step_options(&state.db, id, None).await;
    let bitmaps = fetch_available_bitmaps(&state.db, id).await;

    HtmlTemplate(StepBranchTemplate {
        user,
        csrf_token,
        automation_id: id,
        step_id: None,
        label: String::new(),
        post_delay_seconds: 0.0,
        condition_type,
        x: query.x,
        y: query.y,
        expected_r: Some(0),
        expected_g: Some(0),
        expected_b: Some(0),
        tolerance: Some(10),
        reference_bitmap_id: query.reference_bitmap_id,
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
        match_threshold: Some(0.95),
        on_match_step_id: None,
        on_no_match_step_id: None,
        steps,
        bitmaps,
        error: None,
        is_edit: false,
    })
}

/// GET /automations/{id}/steps/new/find_bitmap
#[tracing::instrument(skip(state, user))]
pub async fn get_new_find_bitmap_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewFindBitmapQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let bitmaps = fetch_available_bitmaps(&state.db, id).await;
    let variables = fetch_automation_variables(&state.db, id).await;

    HtmlTemplate(StepFindBitmapTemplate {
        user,
        csrf_token,
        automation_id: id,
        step_id: None,
        label: String::new(),
        post_delay_seconds: 0.0,
        reference_bitmap_id: query.reference_bitmap_id,
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
        match_threshold: 0.95,
        output_found_variable_id: None,
        output_x_variable_id: None,
        output_y_variable_id: None,
        bitmaps,
        variables,
        error: None,
        is_edit: false,
    })
}

/// GET /automations/{id}/steps/new/find_pixel_rgb
#[tracing::instrument(skip(state, user))]
pub async fn get_new_find_pixel_rgb_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewFindPixelRgbQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let variables = fetch_automation_variables(&state.db, id).await;

    HtmlTemplate(StepFindPixelRgbTemplate {
        user,
        csrf_token,
        automation_id: id,
        step_id: None,
        label: String::new(),
        post_delay_seconds: 0.0,
        x: query.x,
        y: query.y,
        output_variable_id: None,
        variables,
        error: None,
        is_edit: false,
    })
}

/// GET /automations/{id}/steps/new/mouse_click
#[tracing::instrument(skip(state, user))]
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
#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<MouseClickStepForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type_str = form.step_type.as_deref().unwrap_or("");

    if step_type_str == "branch" {
        let condition_type = form
            .condition_type
            .as_deref()
            .unwrap_or("pixel_rgb")
            .to_string();

        let mut error_msg = None;
        if form.on_match_step_id.filter(|&sid| sid > 0).is_none()
            || form.on_no_match_step_id.filter(|&sid| sid > 0).is_none()
        {
            error_msg = Some(
                "Both match and no-match target steps are required for branch conditions."
                    .to_string(),
            );
        } else if condition_type == "pixel_rgb" && (form.x.is_none() || form.y.is_none()) {
            error_msg = Some(
                "Please provide valid X and Y coordinates for pixel RGB condition.".to_string(),
            );
        } else if condition_type == "bitmap"
            && (form.reference_bitmap_id.is_none() || form.reference_bitmap_id == Some(0))
        {
            error_msg = Some("Please select a reference bitmap for bitmap condition.".to_string());
        }

        if let Some(err) = error_msg {
            let steps = fetch_automation_step_options(&state.db, id, None).await;
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepBranchTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    condition_type,
                    x: form.x,
                    y: form.y,
                    expected_r: form.expected_r,
                    expected_g: form.expected_g,
                    expected_b: form.expected_b,
                    tolerance: form.tolerance,
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold,
                    on_match_step_id: form.on_match_step_id,
                    on_no_match_step_id: form.on_no_match_step_id,
                    steps,
                    bitmaps,
                    error: Some(err),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

        let max_pos_row = sqlx::query(
            "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
        )
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
            "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'branch', $3, $4) RETURNING id",
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
                tracing::error!("Failed to insert branch step: {}", e);
                return Redirect::to(&format!("/automations/{}", id)).into_response();
            }
        };

        let detail_res = if condition_type == "pixel_rgb" {
            sqlx::query(
                r#"
                INSERT INTO step_branches
                    (step_id, automation_id, condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                     reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                     on_match_step_id, on_no_match_step_id)
                VALUES ($1, $2, 'pixel_rgb', $3, $4, $5, $6, $7, $8, NULL, NULL, NULL, NULL, NULL, NULL, $9, $10)
                "#,
            )
            .bind(step_id)
            .bind(id)
            .bind(form.x.unwrap_or(0))
            .bind(form.y.unwrap_or(0))
            .bind(form.expected_r.unwrap_or(0))
            .bind(form.expected_g.unwrap_or(0))
            .bind(form.expected_b.unwrap_or(0))
            .bind(form.tolerance.unwrap_or(10))
            .bind(form.on_match_step_id.filter(|&sid| sid > 0))
            .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
                r#"
                INSERT INTO step_branches
                    (step_id, automation_id, condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                     reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                     on_match_step_id, on_no_match_step_id)
                VALUES ($1, $2, 'bitmap', NULL, NULL, NULL, NULL, NULL, NULL, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(step_id)
            .bind(id)
            .bind(form.reference_bitmap_id.unwrap_or(0))
            .bind(form.search_x)
            .bind(form.search_y)
            .bind(form.search_width)
            .bind(form.search_height)
            .bind(form.match_threshold.unwrap_or(0.95))
            .bind(form.on_match_step_id.filter(|&sid| sid > 0))
            .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
            .execute(&mut *tx)
            .await
        };

        if let Err(e) = detail_res {
            tracing::error!("Failed to insert step_branches: {}", e);
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
                    "step_type": "branch",
                    "condition_type": condition_type,
                    "on_match_step_id": form.on_match_step_id,
                    "on_no_match_step_id": form.on_no_match_step_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    if step_type_str == "find_pixel_rgb" {
        let (final_x, final_y): (Option<i32>, Option<i32>) = (form.x, form.y);

        let variables = fetch_automation_variables(&state.db, id).await;

        if final_x.is_none() || final_y.is_none() {
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindPixelRgbTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x: form.x,
                    y: form.y,
                    output_variable_id: form.output_variable_id,
                    variables,
                    error: Some("Please provide valid X and Y coordinates.".to_string()),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        if let Some(out_var_id) = form.output_variable_id
            && !variables.iter().any(|v| v.id == out_var_id)
        {
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindPixelRgbTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x: form.x,
                    y: form.y,
                    output_variable_id: form.output_variable_id,
                    variables,
                    error: Some(
                        "Selected output variable is invalid for this automation.".to_string(),
                    ),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

        let max_pos_row = sqlx::query(
            "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
        )
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
            "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'find_pixel_rgb', $3, $4) RETURNING id",
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
            "INSERT INTO step_find_pixel_rgb (step_id, x, y, output_variable_id) VALUES ($1, $2, $3, $4)",
        )
        .bind(step_id)
        .bind(final_x.unwrap_or(0))
        .bind(final_y.unwrap_or(0))
        .bind(form.output_variable_id)
        .execute(&mut *tx)
        .await;

        if let Err(e) = detail_res {
            tracing::error!("Failed to insert step_find_pixel_rgb: {}", e);
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
                    "step_type": "find_pixel_rgb",
                    "x": final_x,
                    "y": final_y,
                    "output_variable_id": form.output_variable_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    if step_type_str == "find_bitmap" {
        let ref_bitmap_id = match form.reference_bitmap_id {
            Some(id) if id > 0 => id,
            _ => {
                let bitmaps = fetch_available_bitmaps(&state.db, id).await;
                let variables = fetch_automation_variables(&state.db, id).await;
                return (
                    StatusCode::BAD_REQUEST,
                    HtmlTemplate(StepFindBitmapTemplate {
                        user,
                        csrf_token,
                        automation_id: id,
                        step_id: None,
                        label: form.label.unwrap_or_default(),
                        post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                        reference_bitmap_id: form.reference_bitmap_id,
                        search_x: form.search_x,
                        search_y: form.search_y,
                        search_width: form.search_width,
                        search_height: form.search_height,
                        match_threshold: form.match_threshold.unwrap_or(0.95),
                        output_found_variable_id: form.output_found_variable_id,
                        output_x_variable_id: form.output_x_variable_id,
                        output_y_variable_id: form.output_y_variable_id,
                        bitmaps,
                        variables,
                        error: Some("Please select a reference bitmap.".to_string()),
                        is_edit: false,
                    }),
                )
                    .into_response();
            }
        };

        let variables = fetch_automation_variables(&state.db, id).await;

        if let Some(var_id) = form.output_found_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Is Found' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        if let Some(var_id) = form.output_x_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Found X' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        if let Some(var_id) = form.output_y_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: None,
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Found Y' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: false,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);
        let match_threshold = form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0);

        let max_pos_row = sqlx::query(
            "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
        )
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
            "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'find_bitmap', $3, $4) RETURNING id",
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
            r#"
            INSERT INTO step_find_bitmap
                (step_id, reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold, output_found_variable_id, output_x_variable_id, output_y_variable_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(step_id)
        .bind(ref_bitmap_id)
        .bind(form.search_x)
        .bind(form.search_y)
        .bind(form.search_width)
        .bind(form.search_height)
        .bind(match_threshold)
        .bind(form.output_found_variable_id)
        .bind(form.output_x_variable_id)
        .bind(form.output_y_variable_id)
        .execute(&mut *tx)
        .await;

        if let Err(e) = detail_res {
            tracing::error!("Failed to insert step_find_bitmap: {}", e);
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
                    "step_type": "find_bitmap",
                    "reference_bitmap_id": ref_bitmap_id,
                    "search_x": form.search_x,
                    "search_y": form.search_y,
                    "search_width": form.search_width,
                    "search_height": form.search_height,
                    "match_threshold": match_threshold,
                    "output_found_variable_id": form.output_found_variable_id,
                    "output_x_variable_id": form.output_x_variable_id,
                    "output_y_variable_id": form.output_y_variable_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let is_mouse_click = step_type_str == "mouse_click"
        || (form.x_mode.is_some() || form.x_variable_id.is_some() || form.y_variable_id.is_some());

    if is_mouse_click {
        let x_res = validate_literal_or_reference(
            form.x_mode.as_deref(),
            form.x,
            form.x_variable_id,
            "X coordinate",
        );
        let y_res = validate_literal_or_reference(
            form.y_mode.as_deref(),
            form.y,
            form.y_variable_id,
            "Y coordinate",
        );

        let (final_x, final_x_var, final_y, final_y_var) = match (x_res, y_res) {
            (Ok((fx, fx_v)), Ok((fy, fy_v))) => (fx, fx_v, fy, fy_v),
            (Err(e), _) | (_, Err(e)) => {
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
                        x_mode: form.x_mode.unwrap_or_else(|| "fixed".to_string()),
                        x: form.x,
                        x_variable_id: form.x_variable_id,
                        y_mode: form.y_mode.unwrap_or_else(|| "fixed".to_string()),
                        y: form.y,
                        y_variable_id: form.y_variable_id,
                        button: match form.button.as_deref() {
                            Some("right") => "right",
                            Some("middle") => "middle",
                            _ => "left",
                        }
                        .to_string(),
                        click_type: match form.click_type.as_deref() {
                            Some("double") => "double",
                            _ => "single",
                        }
                        .to_string(),
                        variables,
                        error: Some(e),
                        is_edit: false,
                    }),
                )
                    .into_response();
            }
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

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

        let max_pos_row = sqlx::query(
            "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
        )
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
    let label = form
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s: &&str| !s.is_empty())
        .map(String::from);

    let max_pos_row = sqlx::query(
        "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
    )
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

    let detail_res =
        sqlx::query("INSERT INTO step_key_presses (step_id, key_combo) VALUES ($1, $2)")
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
#[tracing::instrument(skip(state, user))]
pub async fn get_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
    Query(query): Query<NewFindBitmapQuery>,
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
    } else if step_type == "find_bitmap" {
        let fb_row = sqlx::query(
            "SELECT reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold, output_found_variable_id, output_x_variable_id, output_y_variable_id FROM step_find_bitmap WHERE step_id = $1",
        )
        .bind(sid)
        .fetch_optional(&state.db)
        .await;

        let (
            db_reference_bitmap_id,
            db_search_x,
            db_search_y,
            db_search_width,
            db_search_height,
            match_threshold,
            output_found_variable_id,
            output_x_variable_id,
            output_y_variable_id,
        ) = match fb_row {
            Ok(Some(r)) => (
                r.get::<Option<i64>, _>("reference_bitmap_id"),
                r.get::<Option<i32>, _>("search_x"),
                r.get::<Option<i32>, _>("search_y"),
                r.get::<Option<i32>, _>("search_width"),
                r.get::<Option<i32>, _>("search_height"),
                r.get::<f32, _>("match_threshold"),
                r.get::<Option<i64>, _>("output_found_variable_id"),
                r.get::<Option<i64>, _>("output_x_variable_id"),
                r.get::<Option<i64>, _>("output_y_variable_id"),
            ),
            _ => (None, None, None, None, None, 0.95, None, None, None),
        };

        let reference_bitmap_id = query.reference_bitmap_id.or(db_reference_bitmap_id);
        let search_x = query.search_x.or(db_search_x);
        let search_y = query.search_y.or(db_search_y);
        let search_width = query.search_width.or(db_search_width);
        let search_height = query.search_height.or(db_search_height);

        let bitmaps = fetch_available_bitmaps(&state.db, id).await;
        let variables = fetch_automation_variables(&state.db, id).await;

        HtmlTemplate(StepFindBitmapTemplate {
            user,
            csrf_token,
            automation_id: id,
            step_id: Some(sid),
            label: label.unwrap_or_default(),
            post_delay_seconds: post_delay_ms as f64 / 1000.0,
            reference_bitmap_id,
            search_x,
            search_y,
            search_width,
            search_height,
            match_threshold,
            output_found_variable_id,
            output_x_variable_id,
            output_y_variable_id,
            bitmaps,
            variables,
            error: None,
            is_edit: true,
        })
        .into_response()
    } else if step_type == "branch" {
        let branch_row = sqlx::query(
            r#"
            SELECT
                condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                on_match_step_id, on_no_match_step_id
            FROM step_branches
            WHERE step_id = $1
            "#,
        )
        .bind(sid)
        .fetch_optional(&state.db)
        .await;

        let (
            db_condition_type,
            db_x,
            db_y,
            db_expected_r,
            db_expected_g,
            db_expected_b,
            db_tolerance,
            db_reference_bitmap_id,
            db_search_x,
            db_search_y,
            db_search_width,
            db_search_height,
            db_match_threshold,
            on_match_step_id,
            on_no_match_step_id,
        ) = match branch_row {
            Ok(Some(r)) => (
                r.get::<String, _>("condition_type"),
                r.get::<Option<i32>, _>("x"),
                r.get::<Option<i32>, _>("y"),
                r.get::<Option<i16>, _>("expected_r"),
                r.get::<Option<i16>, _>("expected_g"),
                r.get::<Option<i16>, _>("expected_b"),
                r.get::<Option<i16>, _>("tolerance"),
                r.get::<Option<i64>, _>("reference_bitmap_id"),
                r.get::<Option<i32>, _>("search_x"),
                r.get::<Option<i32>, _>("search_y"),
                r.get::<Option<i32>, _>("search_width"),
                r.get::<Option<i32>, _>("search_height"),
                r.get::<Option<f32>, _>("match_threshold"),
                r.get::<Option<i64>, _>("on_match_step_id"),
                r.get::<Option<i64>, _>("on_no_match_step_id"),
            ),
            _ => (
                "pixel_rgb".to_string(),
                None,
                None,
                Some(0),
                Some(0),
                Some(0),
                Some(10),
                None,
                None,
                None,
                None,
                None,
                Some(0.95),
                None,
                None,
            ),
        };

        let branch_query = NewBranchQuery {
            condition_type: None,
            x: query.search_x,
            y: query.search_y,
            reference_bitmap_id: query.reference_bitmap_id,
            search_x: query.search_x,
            search_y: query.search_y,
            search_width: query.search_width,
            search_height: query.search_height,
        };

        let x = branch_query.x.or(db_x);
        let y = branch_query.y.or(db_y);
        let reference_bitmap_id = branch_query.reference_bitmap_id.or(db_reference_bitmap_id);
        let search_x = branch_query.search_x.or(db_search_x);
        let search_y = branch_query.search_y.or(db_search_y);
        let search_width = branch_query.search_width.or(db_search_width);
        let search_height = branch_query.search_height.or(db_search_height);

        let steps = fetch_automation_step_options(&state.db, id, Some(sid)).await;
        let bitmaps = fetch_available_bitmaps(&state.db, id).await;

        HtmlTemplate(StepBranchTemplate {
            user,
            csrf_token,
            automation_id: id,
            step_id: Some(sid),
            label: label.unwrap_or_default(),
            post_delay_seconds: post_delay_ms as f64 / 1000.0,
            condition_type: db_condition_type,
            x,
            y,
            expected_r: db_expected_r,
            expected_g: db_expected_g,
            expected_b: db_expected_b,
            tolerance: db_tolerance,
            reference_bitmap_id,
            search_x,
            search_y,
            search_width,
            search_height,
            match_threshold: db_match_threshold,
            on_match_step_id,
            on_no_match_step_id,
            steps,
            bitmaps,
            error: None,
            is_edit: true,
        })
        .into_response()
    } else if step_type == "find_pixel_rgb" {
        let fp_row = sqlx::query(
            "SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1",
        )
        .bind(sid)
        .fetch_optional(&state.db)
        .await;

        let (x, y, output_variable_id) = match fp_row {
            Ok(Some(r)) => (
                r.get::<Option<i32>, _>("x"),
                r.get::<Option<i32>, _>("y"),
                r.get::<Option<i64>, _>("output_variable_id"),
            ),
            _ => (None, None, None),
        };

        let variables = fetch_automation_variables(&state.db, id).await;

        HtmlTemplate(StepFindPixelRgbTemplate {
            user,
            csrf_token,
            automation_id: id,
            step_id: Some(sid),
            label: label.unwrap_or_default(),
            post_delay_seconds: post_delay_ms as f64 / 1000.0,
            x,
            y,
            output_variable_id,
            variables,
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
            _ => (
                None,
                None,
                None,
                None,
                "left".to_string(),
                "single".to_string(),
            ),
        };

        let x_mode = if x_var_id.is_some() {
            "variable"
        } else {
            "fixed"
        }
        .to_string();
        let y_mode = if y_var_id.is_some() {
            "variable"
        } else {
            "fixed"
        }
        .to_string();

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
#[tracing::instrument(skip(state, user, form))]
pub async fn post_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
    CsrfForm(form): CsrfForm<MouseClickStepForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type_row =
        sqlx::query("SELECT step_type FROM automation_steps WHERE id = $1 AND automation_id = $2")
            .bind(sid)
            .bind(id)
            .fetch_optional(&state.db)
            .await;

    let step_type: String = match step_type_row {
        Ok(Some(r)) => r.get("step_type"),
        _ => return Redirect::to(&format!("/automations/{}", id)).into_response(),
    };

    if step_type == "branch" {
        let condition_type = form
            .condition_type
            .as_deref()
            .unwrap_or("pixel_rgb")
            .to_string();

        let mut error_msg = None;
        if form.on_match_step_id.filter(|&sid| sid > 0).is_none()
            || form.on_no_match_step_id.filter(|&sid| sid > 0).is_none()
        {
            error_msg = Some(
                "Both match and no-match target steps are required for branch conditions."
                    .to_string(),
            );
        } else if condition_type == "pixel_rgb" && (form.x.is_none() || form.y.is_none()) {
            error_msg = Some(
                "Please provide valid X and Y coordinates for pixel RGB condition.".to_string(),
            );
        } else if condition_type == "bitmap"
            && (form.reference_bitmap_id.is_none() || form.reference_bitmap_id == Some(0))
        {
            error_msg = Some("Please select a reference bitmap for bitmap condition.".to_string());
        }

        if let Some(err) = error_msg {
            let steps = fetch_automation_step_options(&state.db, id, Some(sid)).await;
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepBranchTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    condition_type,
                    x: form.x,
                    y: form.y,
                    expected_r: form.expected_r,
                    expected_g: form.expected_g,
                    expected_b: form.expected_b,
                    tolerance: form.tolerance,
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold,
                    on_match_step_id: form.on_match_step_id,
                    on_no_match_step_id: form.on_no_match_step_id,
                    steps,
                    bitmaps,
                    error: Some(err),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

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

        let detail_res = if condition_type == "pixel_rgb" {
            sqlx::query(
                r#"
                UPDATE step_branches
                SET condition_type = 'pixel_rgb',
                    x = $1, y = $2, expected_r = $3, expected_g = $4, expected_b = $5, tolerance = $6,
                    reference_bitmap_id = NULL, search_x = NULL, search_y = NULL, search_width = NULL, search_height = NULL, match_threshold = NULL,
                    on_match_step_id = $7, on_no_match_step_id = $8, automation_id = $9
                WHERE step_id = $10
                "#,
            )
            .bind(form.x.unwrap_or(0))
            .bind(form.y.unwrap_or(0))
            .bind(form.expected_r.unwrap_or(0))
            .bind(form.expected_g.unwrap_or(0))
            .bind(form.expected_b.unwrap_or(0))
            .bind(form.tolerance.unwrap_or(10))
            .bind(form.on_match_step_id.filter(|&sid| sid > 0))
            .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
            .bind(id)
            .bind(sid)
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
                r#"
                UPDATE step_branches
                SET condition_type = 'bitmap',
                    x = NULL, y = NULL, expected_r = NULL, expected_g = NULL, expected_b = NULL, tolerance = NULL,
                    reference_bitmap_id = $1, search_x = $2, search_y = $3, search_width = $4, search_height = $5, match_threshold = $6,
                    on_match_step_id = $7, on_no_match_step_id = $8, automation_id = $9
                WHERE step_id = $10
                "#,
            )
            .bind(form.reference_bitmap_id.unwrap_or(0))
            .bind(form.search_x)
            .bind(form.search_y)
            .bind(form.search_width)
            .bind(form.search_height)
            .bind(form.match_threshold.unwrap_or(0.95))
            .bind(form.on_match_step_id.filter(|&sid| sid > 0))
            .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
            .bind(id)
            .bind(sid)
            .execute(&mut *tx)
            .await
        };

        if let Err(e) = detail_res {
            tracing::error!("Failed to update step_branches: {}", e);
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
                "update_step",
                "automation_step",
                Some(sid),
                Some(serde_json::json!({
                    "automation_id": id,
                    "step_type": "branch",
                    "condition_type": condition_type,
                    "on_match_step_id": form.on_match_step_id,
                    "on_no_match_step_id": form.on_no_match_step_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    if step_type == "find_pixel_rgb" {
        let (final_x, final_y): (Option<i32>, Option<i32>) = (form.x, form.y);

        let variables = fetch_automation_variables(&state.db, id).await;

        if final_x.is_none() || final_y.is_none() {
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindPixelRgbTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x: form.x,
                    y: form.y,
                    output_variable_id: form.output_variable_id,
                    variables,
                    error: Some("Please provide valid X and Y coordinates.".to_string()),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        if let Some(out_var_id) = form.output_variable_id
            && !variables.iter().any(|v| v.id == out_var_id)
        {
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindPixelRgbTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    x: form.x,
                    y: form.y,
                    output_variable_id: form.output_variable_id,
                    variables,
                    error: Some(
                        "Selected output variable is invalid for this automation.".to_string(),
                    ),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

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
            "UPDATE step_find_pixel_rgb SET x = $1, y = $2, output_variable_id = $3 WHERE step_id = $4",
        )
        .bind(final_x.unwrap_or(0))
        .bind(final_y.unwrap_or(0))
        .bind(form.output_variable_id)
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
                    "step_type": "find_pixel_rgb",
                    "x": final_x,
                    "y": final_y,
                    "output_variable_id": form.output_variable_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    if step_type == "find_bitmap" {
        let ref_bitmap_id = match form.reference_bitmap_id {
            Some(id) if id > 0 => id,
            _ => {
                let bitmaps = fetch_available_bitmaps(&state.db, id).await;
                let variables = fetch_automation_variables(&state.db, id).await;
                return (
                    StatusCode::BAD_REQUEST,
                    HtmlTemplate(StepFindBitmapTemplate {
                        user,
                        csrf_token,
                        automation_id: id,
                        step_id: Some(sid),
                        label: form.label.unwrap_or_default(),
                        post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                        reference_bitmap_id: form.reference_bitmap_id,
                        search_x: form.search_x,
                        search_y: form.search_y,
                        search_width: form.search_width,
                        search_height: form.search_height,
                        match_threshold: form.match_threshold.unwrap_or(0.95),
                        output_found_variable_id: form.output_found_variable_id,
                        output_x_variable_id: form.output_x_variable_id,
                        output_y_variable_id: form.output_y_variable_id,
                        bitmaps,
                        variables,
                        error: Some("Please select a reference bitmap.".to_string()),
                        is_edit: true,
                    }),
                )
                    .into_response();
            }
        };

        let variables = fetch_automation_variables(&state.db, id).await;

        if let Some(var_id) = form.output_found_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Is Found' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        if let Some(var_id) = form.output_x_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Found X' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        if let Some(var_id) = form.output_y_variable_id
            && !variables.iter().any(|v| v.id == var_id)
        {
            let bitmaps = fetch_available_bitmaps(&state.db, id).await;
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(StepFindBitmapTemplate {
                    user,
                    csrf_token,
                    automation_id: id,
                    step_id: Some(sid),
                    label: form.label.unwrap_or_default(),
                    post_delay_seconds: form.post_delay_seconds.unwrap_or(0.0),
                    reference_bitmap_id: form.reference_bitmap_id,
                    search_x: form.search_x,
                    search_y: form.search_y,
                    search_width: form.search_width,
                    search_height: form.search_height,
                    match_threshold: form.match_threshold.unwrap_or(0.95),
                    output_found_variable_id: form.output_found_variable_id,
                    output_x_variable_id: form.output_x_variable_id,
                    output_y_variable_id: form.output_y_variable_id,
                    bitmaps,
                    variables,
                    error: Some(
                        "Selected 'Found Y' output variable is invalid for this automation."
                            .to_string(),
                    ),
                    is_edit: true,
                }),
            )
                .into_response();
        }

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);
        let match_threshold = form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0);

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
            r#"
            UPDATE step_find_bitmap
            SET reference_bitmap_id = $1, search_x = $2, search_y = $3, search_width = $4, search_height = $5,
                match_threshold = $6, output_found_variable_id = $7, output_x_variable_id = $8, output_y_variable_id = $9
            WHERE step_id = $10
            "#,
        )
        .bind(ref_bitmap_id)
        .bind(form.search_x)
        .bind(form.search_y)
        .bind(form.search_width)
        .bind(form.search_height)
        .bind(match_threshold)
        .bind(form.output_found_variable_id)
        .bind(form.output_x_variable_id)
        .bind(form.output_y_variable_id)
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
                    "step_type": "find_bitmap",
                    "reference_bitmap_id": ref_bitmap_id,
                    "search_x": form.search_x,
                    "search_y": form.search_y,
                    "search_width": form.search_width,
                    "search_height": form.search_height,
                    "match_threshold": match_threshold,
                    "output_found_variable_id": form.output_found_variable_id,
                    "output_x_variable_id": form.output_x_variable_id,
                    "output_y_variable_id": form.output_y_variable_id
                })),
            )
            .await;
        }

        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    if step_type == "mouse_click" {
        let x_res = validate_literal_or_reference(
            form.x_mode.as_deref(),
            form.x,
            form.x_variable_id,
            "X coordinate",
        );
        let y_res = validate_literal_or_reference(
            form.y_mode.as_deref(),
            form.y,
            form.y_variable_id,
            "Y coordinate",
        );

        let (final_x, final_x_var, final_y, final_y_var) = match (x_res, y_res) {
            (Ok((fx, fx_v)), Ok((fy, fy_v))) => (fx, fx_v, fy, fy_v),
            (Err(e), _) | (_, Err(e)) => {
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
                        x_mode: form.x_mode.unwrap_or_else(|| "fixed".to_string()),
                        x: form.x,
                        x_variable_id: form.x_variable_id,
                        y_mode: form.y_mode.unwrap_or_else(|| "fixed".to_string()),
                        y: form.y,
                        y_variable_id: form.y_variable_id,
                        button: match form.button.as_deref() {
                            Some("right") => "right",
                            Some("middle") => "middle",
                            _ => "left",
                        }
                        .to_string(),
                        click_type: match form.click_type.as_deref() {
                            Some("double") => "double",
                            _ => "single",
                        }
                        .to_string(),
                        variables,
                        error: Some(e),
                        is_edit: true,
                    }),
                )
                    .into_response();
            }
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

        let post_delay_ms = ((form.post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32;
        let label = form
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s: &&str| !s.is_empty())
            .map(String::from);

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
    let label = form
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s: &&str| !s.is_empty())
        .map(String::from);

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
#[tracing::instrument(skip(state, user))]
pub async fn post_move_step_up_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if user.role.can_edit()
        && let Err(e) = reorder_step(&state.db, id, sid, true).await
    {
        tracing::error!("Failed to move step {} up in automation {}: {}", sid, id, e);
    }
    Redirect::to(&format!("/automations/{}", id))
}

/// POST /automations/{id}/steps/{sid}/move-down
#[tracing::instrument(skip(state, user))]
pub async fn post_move_step_down_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if user.role.can_edit()
        && let Err(e) = reorder_step(&state.db, id, sid, false).await
    {
        tracing::error!(
            "Failed to move step {} down in automation {}: {}",
            sid,
            id,
            e
        );
    }
    Redirect::to(&format!("/automations/{}", id))
}

/// Helper function to reorder step up or down
pub async fn reorder_step(
    db: &PgPool,
    automation_id: i64,
    step_id: i64,
    is_up: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    // Lock the automation row to serialize concurrent step reordering / mutations
    sqlx::query("SELECT id FROM automations WHERE id = $1 FOR UPDATE")
        .bind(automation_id)
        .fetch_optional(&mut *tx)
        .await?;

    let steps = sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(&mut *tx)
        .await?;

    let current_idx = steps.iter().position(|r| r.get::<i64, _>("id") == step_id);
    let idx = match current_idx {
        Some(i) => i,
        None => return Ok(()),
    };

    if is_up && idx == 0 {
        return Ok(()); // Already at top
    }
    if !is_up && idx == steps.len() - 1 {
        return Ok(()); // Already at bottom
    }

    let target_idx = if is_up { idx - 1 } else { idx + 1 };

    let pos_curr: f64 = steps[idx].get("position");
    let pos_target: f64 = steps[target_idx].get("position");
    let target_id: i64 = steps[target_idx].get("id");

    // Swap positions within the transaction
    sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
        .bind(pos_target)
        .bind(step_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
        .bind(pos_curr)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(automation_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Check gap precision & compact if gaps < 0.0001
    check_and_compact_positions(db, automation_id).await?;
    Ok(())
}

/// POST /automations/{id}/steps/{sid}/delete
#[tracing::instrument(skip(state, user))]
pub async fn post_delete_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    // Check if step is referenced in task_run_steps execution history
    let has_history: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_run_steps WHERE step_id = $1)")
            .bind(sid)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);

    let res = if has_history {
        // Soft delete step to preserve run history
        sqlx::query(
            "UPDATE automation_steps SET deleted_at = now() WHERE id = $1 AND automation_id = $2",
        )
        .bind(sid)
        .bind(id)
        .execute(&state.db)
        .await
    } else {
        // Hard delete step
        sqlx::query("DELETE FROM automation_steps WHERE id = $1 AND automation_id = $2")
            .bind(sid)
            .bind(id)
            .execute(&state.db)
            .await
    };

    if let Err(e) = &res {
        tracing::error!(
            "Failed to delete step {} from automation {}: {}",
            sid,
            id,
            e
        );
    } else {
        let _ = sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await;

        let _ = log_audit(
            &state.db,
            Some(user.id),
            if has_history {
                "soft_delete_step"
            } else {
                "delete_step"
            },
            "automation_step",
            Some(sid),
            Some(serde_json::json!({ "automation_id": id, "soft_deleted": has_history })),
        )
        .await;

        if let Err(e) = check_and_compact_positions(&state.db, id).await {
            tracing::error!(
                "Failed to compact positions after deleting step {}: {}",
                sid,
                e
            );
        }
    }

    Redirect::to(&format!("/automations/{}", id)).into_response()
}

/// Compact positions to 10.0, 20.0, 30.0... if gaps between adjacent steps are too narrow (< 0.0001)
#[tracing::instrument(skip(db))]
pub async fn check_and_compact_positions(
    db: &PgPool,
    automation_id: i64,
) -> Result<(), sqlx::Error> {
    let steps = sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(db)
        .await?;

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
        compact_positions(db, automation_id).await?;
    }

    Ok(())
}

#[tracing::instrument(skip(db))]
pub async fn compact_positions(db: &PgPool, automation_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    // Lock the automation row to serialize concurrent step operations
    sqlx::query("SELECT id FROM automations WHERE id = $1 FOR UPDATE")
        .bind(automation_id)
        .fetch_optional(&mut *tx)
        .await?;

    let steps = sqlx::query("SELECT id FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC")
        .bind(automation_id)
        .fetch_all(&mut *tx)
        .await?;

    for (i, row) in steps.iter().enumerate() {
        let step_id: i64 = row.get("id");
        let new_pos = (i as f64 + 1.0) * 10.0;
        sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
            .bind(new_pos)
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_literal_or_reference() {
        // 1. Mode "fixed" with literal value present -> Ok((Some(val), None))
        let r1 = validate_literal_or_reference(Some("fixed"), Some(824), None, "X coordinate");
        assert_eq!(r1, Ok((Some(824), None)));

        // 2. Mode "fixed" with missing literal -> Err
        let r2 = validate_literal_or_reference::<i32>(Some("fixed"), None, None, "X coordinate");
        assert!(r2.is_err());
        assert!(
            r2.unwrap_err()
                .contains("Please provide a fixed value for X coordinate.")
        );

        // 3. Mode "variable" with valid variable ID (> 0) -> Ok((None, Some(var_id)))
        let r3 =
            validate_literal_or_reference::<i32>(Some("variable"), None, Some(10), "X coordinate");
        assert_eq!(r3, Ok((None, Some(10))));

        // 4. Mode "variable" with invalid variable ID (None or <= 0) -> Err
        let r4 =
            validate_literal_or_reference::<i32>(Some("variable"), None, Some(0), "X coordinate");
        assert!(r4.is_err());
        assert!(
            r4.unwrap_err()
                .contains("Please select a valid variable for X coordinate.")
        );

        let r5 = validate_literal_or_reference::<i32>(Some("variable"), None, None, "X coordinate");
        assert!(r5.is_err());

        // 5. Unspecified mode with only literal -> Ok((Some(val), None))
        let r6 = validate_literal_or_reference(None, Some(100), None, "X coordinate");
        assert_eq!(r6, Ok((Some(100), None)));

        // 6. Unspecified mode with only variable ID -> Ok((None, Some(var_id)))
        let r7 = validate_literal_or_reference::<i32>(None, None, Some(15), "X coordinate");
        assert_eq!(r7, Ok((None, Some(15))));

        // 7. Unspecified mode with both literal and variable ID -> Err
        let r8 = validate_literal_or_reference(None, Some(100), Some(15), "X coordinate");
        assert!(r8.is_err());
        assert!(
            r8.unwrap_err()
                .contains("Cannot specify both a fixed value and a variable for X coordinate.")
        );

        // 8. Unspecified mode with neither literal nor variable ID -> Err
        let r9 = validate_literal_or_reference::<i32>(None, None, None, "X coordinate");
        assert!(r9.is_err());
        assert!(
            r9.unwrap_err()
                .contains("Please specify either a fixed value or a variable for X coordinate.")
        );
    }

    #[test]
    fn test_generate_mouse_click_summary() {
        // Fixed coordinates
        let s1 = generate_mouse_click_summary(Some(824), Some(391), None, None, "left", "single");
        assert_eq!(s1, "Click (824, 391) [left, single]");

        // Variable X, Fixed Y
        let s2 = generate_mouse_click_summary(
            None,
            Some(391),
            Some("target_x"),
            None,
            "right",
            "double",
        );
        assert_eq!(s2, "Click («target_x», 391) [right, double]");

        // Variable X and Y
        let s3 = generate_mouse_click_summary(
            None,
            None,
            Some("target_x"),
            Some("target_y"),
            "middle",
            "single",
        );
        assert_eq!(s3, "Click («target_x», «target_y») [middle, single]");

        // Variable X with no name fallback
        let s4 = generate_mouse_click_summary(None, Some(100), None, None, "left", "single");
        assert_eq!(s4, "Click (variable, 100) [left, single]");
    }

    #[test]
    fn test_generate_find_pixel_rgb_summary() {
        // Without output variable
        let s1 = generate_find_pixel_rgb_summary(100, 200, None);
        assert_eq!(s1, "Read pixel at (100, 200)");

        // With output variable
        let s2 = generate_find_pixel_rgb_summary(100, 200, Some("bg_color"));
        assert_eq!(s2, "Read pixel at (100, 200) → store as «bg_color»");

        // With empty output variable string
        let s3 = generate_find_pixel_rgb_summary(150, 250, Some("   "));
        assert_eq!(s3, "Read pixel at (150, 250)");
    }

    #[test]
    fn test_generate_find_bitmap_summary() {
        // Without output variable
        let s1 = generate_find_bitmap_summary("login_button", None);
        assert_eq!(s1, "Search for «login_button» on screen");

        // With output variable
        let s2 = generate_find_bitmap_summary("login_button", Some("found_login"));
        assert_eq!(
            s2,
            "Search for «login_button» on screen → store as «found_login»"
        );

        // With whitespace output variable name
        let s3 = generate_find_bitmap_summary("login_button", Some("   "));
        assert_eq!(s3, "Search for «login_button» on screen");
    }

    #[test]
    fn test_generate_branch_summary() {
        // Pixel RGB branch condition
        let s1 = generate_branch_summary(
            "pixel_rgb",
            Some(824),
            Some(391),
            Some(40),
            Some(180),
            Some(60),
            Some(10),
            None,
            Some("Step 12 (Success)"),
            Some("Step 10 (Retry)"),
        );
        assert_eq!(
            s1,
            "BRANCH: if pixel at (824, 391) ≈ RGB(40,180,60) ±10 → go to Step 12 (Success), else → go to Step 10 (Retry)"
        );

        // Bitmap branch condition
        let s2 = generate_branch_summary(
            "bitmap",
            None,
            None,
            None,
            None,
            None,
            None,
            Some("submit_btn"),
            Some("Step 5 (Click)"),
            Some("Next Step"),
        );
        assert_eq!(
            s2,
            "BRANCH: if bitmap «submit_btn» found → go to Step 5 (Click), else → go to Next Step"
        );

        // Fallback targets and values when None
        let s3 = generate_branch_summary(
            "pixel_rgb",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            s3,
            "BRANCH: if pixel at (0, 0) ≈ RGB(0,0,0) ±0 → go to Next Step, else → go to Next Step"
        );

        let s4 = generate_branch_summary(
            "bitmap", None, None, None, None, None, None, None, None, None,
        );
        assert_eq!(
            s4,
            "BRANCH: if bitmap «bitmap» found → go to Next Step, else → go to Next Step"
        );
    }

    #[test]
    fn test_find_bitmap_query_overrides() {
        let q = NewFindBitmapQuery {
            reference_bitmap_id: Some(42),
            search_x: Some(10),
            search_y: Some(20),
            search_width: Some(300),
            search_height: Some(400),
        };

        let db_ref_id: Option<i64> = None;
        let db_x: Option<i32> = None;

        assert_eq!(q.reference_bitmap_id.or(db_ref_id), Some(42));
        assert_eq!(q.search_x.or(db_x), Some(10));
        assert_eq!(q.search_y, Some(20));
        assert_eq!(q.search_width, Some(300));
        assert_eq!(q.search_height, Some(400));
    }

    #[test]
    fn test_step_mouse_click_template_rendering_literal_and_reference_toggles() {
        use crate::auth::UserRole;
        use askama::Template;

        let dummy_user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf".to_string(),
            must_change_password: false,
        };

        let vars = vec![
            VariableOption {
                id: 10,
                name: "var_x".to_string(),
                var_type: "int".to_string(),
            },
            VariableOption {
                id: 11,
                name: "var_y".to_string(),
                var_type: "int".to_string(),
            },
        ];

        // 1. Fixed mode (Literal values)
        let tmpl_fixed = StepMouseClickTemplate {
            user: dummy_user.clone(),
            csrf_token: "test_csrf".to_string(),
            automation_id: 1,
            step_id: None,
            label: "Test Click".to_string(),
            post_delay_seconds: 0.5,
            x_mode: "fixed".to_string(),
            x: Some(824),
            x_variable_id: None,
            y_mode: "fixed".to_string(),
            y: Some(391),
            y_variable_id: None,
            button: "left".to_string(),
            click_type: "single".to_string(),
            variables: vars.clone(),
            error: None,
            is_edit: false,
        };

        let rendered_fixed = tmpl_fixed.render().unwrap();
        assert!(rendered_fixed.contains("<select name=\"x_mode\""));
        assert!(rendered_fixed.contains("<option value=\"fixed\" selected>Fixed Value</option>"));
        assert!(rendered_fixed.contains("value=\"824\""));

        // 2. Variable mode (Reference values)
        let tmpl_var = StepMouseClickTemplate {
            user: dummy_user,
            csrf_token: "test_csrf".to_string(),
            automation_id: 1,
            step_id: Some(5),
            label: "Edit Click".to_string(),
            post_delay_seconds: 1.0,
            x_mode: "variable".to_string(),
            x: None,
            x_variable_id: Some(10),
            y_mode: "variable".to_string(),
            y: None,
            y_variable_id: Some(11),
            button: "right".to_string(),
            click_type: "double".to_string(),
            variables: vars,
            error: None,
            is_edit: true,
        };

        let rendered_var = tmpl_var.render().unwrap();
        assert!(rendered_var.contains("<select name=\"x_mode\""));
        assert!(
            rendered_var.contains("<option value=\"variable\" selected>From Variable</option>")
        );
        assert!(rendered_var.contains("var_x (int)"));
        assert!(rendered_var.contains("var_y (int)"));
    }

    #[test]
    fn test_step_find_pixel_rgb_template_rendering() {
        use crate::auth::UserRole;
        use askama::Template;

        let dummy_user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf".to_string(),
            must_change_password: false,
        };

        let vars = vec![VariableOption {
            id: 10,
            name: "bg_color".to_string(),
            var_type: "color".to_string(),
        }];

        let tmpl = StepFindPixelRgbTemplate {
            user: dummy_user,
            csrf_token: "test_csrf".to_string(),
            automation_id: 1,
            step_id: Some(2),
            label: "Sample Pixel".to_string(),
            post_delay_seconds: 0.5,
            x: Some(100),
            y: Some(200),
            output_variable_id: Some(10),
            variables: vars,
            error: None,
            is_edit: true,
        };

        let rendered = tmpl.render().unwrap();
        assert!(rendered.contains("<select name=\"output_variable_id\""));
        assert!(rendered.contains("bg_color (color)"));
        assert!(rendered.contains("selected"));
    }

    #[test]
    fn test_step_find_bitmap_template_rendering_with_output_variables() {
        use crate::auth::UserRole;
        use askama::Template;

        let dummy_user = AuthUser {
            id: 1,
            username: "admin".to_string(),
            display_name: "Admin User".to_string(),
            role: UserRole::Admin,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf".to_string(),
            must_change_password: false,
        };

        let vars = vec![
            VariableOption {
                id: 10,
                name: "is_found".to_string(),
                var_type: "bool".to_string(),
            },
            VariableOption {
                id: 11,
                name: "pos_x".to_string(),
                var_type: "int".to_string(),
            },
            VariableOption {
                id: 12,
                name: "pos_y".to_string(),
                var_type: "int".to_string(),
            },
        ];

        let bitmaps = vec![BitmapOption {
            id: 1,
            name: "login_button.png".to_string(),
            width: 100,
            height: 40,
        }];

        let tmpl = StepFindBitmapTemplate {
            user: dummy_user,
            csrf_token: "test_csrf".to_string(),
            automation_id: 1,
            step_id: Some(3),
            label: "Find Login".to_string(),
            post_delay_seconds: 0.2,
            reference_bitmap_id: Some(1),
            search_x: Some(10),
            search_y: Some(20),
            search_width: Some(300),
            search_height: Some(200),
            match_threshold: 0.95,
            output_found_variable_id: Some(10),
            output_x_variable_id: Some(11),
            output_y_variable_id: Some(12),
            bitmaps,
            variables: vars,
            error: None,
            is_edit: true,
        };

        let rendered = tmpl.render().unwrap();
        assert!(rendered.contains("output_found_variable_id"));
        assert!(rendered.contains("output_x_variable_id"));
        assert!(rendered.contains("output_y_variable_id"));
        assert!(rendered.contains("is_found (bool)"));
        assert!(rendered.contains("pos_x (int)"));
        assert!(rendered.contains("pos_y (int)"));
        assert!(rendered.contains("selected"));
    }

    #[test]
    fn test_find_bitmap_output_variables_validation() {
        let vars = [
            VariableOption {
                id: 10,
                name: "found_var".to_string(),
                var_type: "bool".to_string(),
            },
            VariableOption {
                id: 11,
                name: "x_var".to_string(),
                var_type: "int".to_string(),
            },
        ];

        let valid_found = Some(10);
        let invalid_found = Some(999);
        let valid_x = Some(11);
        let invalid_x = Some(888);

        assert!(vars.iter().any(|v| Some(v.id) == valid_found));
        assert!(!vars.iter().any(|v| Some(v.id) == invalid_found));
        assert!(vars.iter().any(|v| Some(v.id) == valid_x));
        assert!(!vars.iter().any(|v| Some(v.id) == invalid_x));
    }

    #[test]
    fn test_step_description_variable_formatting() {
        // Variable-bound X and Y coordinates
        let summary1 = generate_mouse_click_summary(
            None,
            None,
            Some("button_x"),
            Some("button_y"),
            "left",
            "single",
        );
        assert_eq!(summary1, "Click («button_x», «button_y») [left, single]");

        // Variable-bound X coordinate and fixed Y coordinate
        let summary2 = generate_mouse_click_summary(
            None,
            Some(250),
            Some("target_x"),
            None,
            "right",
            "double",
        );
        assert_eq!(summary2, "Click («target_x», 250) [right, double]");

        // Fixed X coordinate and variable-bound Y coordinate
        let summary3 = generate_mouse_click_summary(
            Some(500),
            None,
            None,
            Some("target_y"),
            "middle",
            "single",
        );
        assert_eq!(summary3, "Click (500, «target_y») [middle, single]");
    }
}
