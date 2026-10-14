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

// ============================================================================
// Helper Functions for Validation, Delay & Step Database Operations
// ============================================================================

#[inline]
pub fn seconds_to_post_delay_ms(post_delay_seconds: Option<f64>) -> i32 {
    ((post_delay_seconds.unwrap_or(0.0).max(0.0)) * 1000.0) as i32
}

#[inline]
fn sanitize_mouse_button(b: Option<&str>) -> &'static str {
    match b {
        Some("right") => "right",
        Some("middle") => "middle",
        _ => "left",
    }
}

#[inline]
fn sanitize_click_type(ct: Option<&str>) -> &'static str {
    match ct {
        Some("double") => "double",
        _ => "single",
    }
}

fn validate_branch_form(form: &MouseClickStepForm, condition_type: &str) -> Option<String> {
    if form.on_match_step_id.filter(|&sid| sid > 0).is_none()
        || form.on_no_match_step_id.filter(|&sid| sid > 0).is_none()
    {
        Some("Both match and no-match target steps are required for branch conditions.".to_string())
    } else if condition_type == "pixel_rgb" && (form.x.is_none() || form.y.is_none()) {
        Some("Please provide valid X and Y coordinates for pixel RGB condition.".to_string())
    } else if condition_type == "bitmap"
        && (form.reference_bitmap_id.is_none() || form.reference_bitmap_id == Some(0))
    {
        Some("Please select a reference bitmap for bitmap condition.".to_string())
    } else {
        None
    }
}

fn validate_find_bitmap_output_vars(
    form: &MouseClickStepForm,
    variables: &[VariableOption],
) -> Result<i64, &'static str> {
    let ref_bitmap_id = match form.reference_bitmap_id {
        Some(id) if id > 0 => id,
        _ => return Err("Please select a reference bitmap."),
    };

    if let Some(var_id) = form.output_found_variable_id
        && !variables.iter().any(|v| v.id == var_id)
    {
        return Err("Selected 'Is Found' output variable is invalid for this automation.");
    }
    if let Some(var_id) = form.output_x_variable_id
        && !variables.iter().any(|v| v.id == var_id)
    {
        return Err("Selected 'Found X' output variable is invalid for this automation.");
    }
    if let Some(var_id) = form.output_y_variable_id
        && !variables.iter().any(|v| v.id == var_id)
    {
        return Err("Selected 'Found Y' output variable is invalid for this automation.");
    }
    if let Some(var_id) = form.output_region_variable_id
        && !variables.iter().any(|v| v.id == var_id)
    {
        return Err("Selected 'Found Region' output variable is invalid for this automation.");
    }

    Ok(ref_bitmap_id)
}

#[inline]
pub fn sanitize_label(label: Option<&str>) -> Option<String> {
    label
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

pub fn validate_literal_or_reference<T>(
    mode: Option<&str>,
    literal: Option<T>,
    variable_id: Option<i64>,
    field_name: &str,
) -> Result<(Option<T>, Option<i64>), String> {
    let var_id = variable_id.filter(|&v| v > 0);
    let is_var = match mode {
        Some("variable") => true,
        Some("fixed") => false,
        _ => match (literal.is_some(), var_id.is_some()) {
            (true, false) => false,
            (false, true) => true,
            (true, true) => {
                return Err(format!(
                    "Cannot specify both a fixed value and a variable for {}.",
                    field_name
                ));
            }
            (false, false) => {
                return Err(format!(
                    "Please specify either a fixed value or a variable for {}.",
                    field_name
                ));
            }
        },
    };

    if is_var {
        var_id
            .map(|v| (None, Some(v)))
            .ok_or_else(|| format!("Please select a valid variable for {}.", field_name))
    } else {
        literal
            .map(|val| (Some(val), None))
            .ok_or_else(|| format!("Please provide a fixed value for {}.", field_name))
    }
}

async fn get_next_step_position<'e, E>(executor: E, automation_id: i64) -> f64
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let max_pos_row = sqlx::query(
        "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(executor)
    .await;

    match max_pos_row {
        Ok(r) => r
            .get::<Option<f64>, _>("max_pos")
            .map(|p| p + 10.0)
            .unwrap_or(10.0),
        _ => 10.0,
    }
}

async fn insert_base_step(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    automation_id: i64,
    step_type: &str,
    label: Option<&str>,
    post_delay_seconds: Option<f64>,
) -> Result<i64, sqlx::Error> {
    let next_pos = get_next_step_position(&mut **tx, automation_id).await;
    let post_delay_ms = seconds_to_post_delay_ms(post_delay_seconds);
    let clean_label = sanitize_label(label);

    let row = sqlx::query(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(automation_id)
    .bind(next_pos)
    .bind(step_type)
    .bind(&clean_label)
    .bind(post_delay_ms)
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.get("id"))
}

async fn update_base_step(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    automation_id: i64,
    step_id: i64,
    label: Option<&str>,
    post_delay_seconds: Option<f64>,
) -> Result<(), sqlx::Error> {
    let post_delay_ms = seconds_to_post_delay_ms(post_delay_seconds);
    let clean_label = sanitize_label(label);

    sqlx::query(
        "UPDATE automation_steps SET label = $1, post_delay_ms = $2, updated_at = now() WHERE id = $3 AND automation_id = $4",
    )
    .bind(&clean_label)
    .bind(post_delay_ms)
    .bind(step_id)
    .bind(automation_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query("UPDATE automations SET updated_at = now() WHERE id = $1")
        .bind(automation_id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

async fn commit_step_tx_and_audit(
    db: &PgPool,
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    action: &str,
    step_id: i64,
    audit_details: serde_json::Value,
) {
    if tx.commit().await.is_ok() {
        let _ = log_audit(
            db,
            Some(user_id),
            action,
            "automation_step",
            Some(step_id),
            Some(audit_details),
        )
        .await;
    }
}

// ============================================================================
// Query & Summary Formatting Helpers
// ============================================================================

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
            Some(StepOption {
                id,
                step_type: row.get("step_type"),
                label: row.get("label"),
                position: row.get("position"),
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
    region_var_name: Option<&str>,
    button: &str,
    click_type: &str,
) -> String {
    use std::fmt::Write;
    let mut summary = String::with_capacity(64);
    if let Some(rname) = region_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(
            summary,
            "Click center of region «{}» [{}, {}]",
            rname, button, click_type
        );
        return summary;
    }

    summary.push_str("Click (");

    fn fmt_coord(coord: Option<i32>, var_name: Option<&str>, buf: &mut String) {
        if let Some(name) = var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, "«{}»", name);
        } else if let Some(val) = coord {
            let _ = write!(buf, "{}", val);
        } else {
            buf.push_str("variable");
        }
    }

    fmt_coord(x, x_var_name, &mut summary);
    summary.push_str(", ");
    fmt_coord(y, y_var_name, &mut summary);
    let _ = write!(summary, ") [{}, {}]", button, click_type);

    summary
}

pub fn generate_find_pixel_rgb_summary(x: i32, y: i32, output_var_name: Option<&str>) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let _ = write!(buf, "Read pixel at ({}, {})", x, y);
    if let Some(name) = output_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(buf, " → store as «{}»", name);
    }
    buf
}

pub fn generate_find_bitmap_summary(
    bitmap_name: &str,
    output_found_var_name: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let _ = write!(buf, "Search for «{}» on screen", bitmap_name);
    if let Some(name) = output_found_var_name
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let _ = write!(buf, " → store as «{}»", name);
    }
    buf
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
    use std::fmt::Write;
    let match_str = match_target.unwrap_or("Next Step");
    let no_match_str = no_match_target.unwrap_or("Next Step");
    let mut buf = String::with_capacity(128);

    if condition_type == "pixel_rgb" {
        let _ = write!(
            buf,
            "BRANCH: if pixel at ({}, {}) ≈ RGB({},{},{}) ±{} → go to {}, else → go to {}",
            x.unwrap_or(0),
            y.unwrap_or(0),
            expected_r.unwrap_or(0),
            expected_g.unwrap_or(0),
            expected_b.unwrap_or(0),
            tolerance.unwrap_or(0),
            match_str,
            no_match_str
        );
    } else {
        let _ = write!(
            buf,
            "BRANCH: if bitmap «{}» found → go to {}, else → go to {}",
            bitmap_name.unwrap_or("bitmap"),
            match_str,
            no_match_str
        );
    }
    buf
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

    let (kp_res, mc_res, fp_res, fb_res, br_res) = tokio::join!(
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
        .map(|r| (r.get("step_id"), r.get("key_combo")))
        .collect();

    let mouse_clicks_map: std::collections::HashMap<
        i64,
        (
            Option<i32>,
            Option<i32>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            String,
        ),
    > = mc_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            (
                r.get("step_id"),
                (
                    r.get("x"),
                    r.get("y"),
                    r.get("x_var_name"),
                    r.get("y_var_name"),
                    r.get("region_var_name"),
                    r.get("button"),
                    r.get("click_type"),
                ),
            )
        })
        .collect();

    let find_pixel_map: std::collections::HashMap<i64, (i32, i32, Option<String>)> = fp_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            (
                r.get("step_id"),
                (r.get("x"), r.get("y"), r.get("var_name")),
            )
        })
        .collect();

    let find_bitmap_map: std::collections::HashMap<i64, (String, Option<String>)> = fb_res
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.get("step_id"), (r.get("bitmap_name"), r.get("var_name"))))
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
            (
                r.get("step_id"),
                (
                    r.get("condition_type"),
                    r.get("x"),
                    r.get("y"),
                    r.get("expected_r"),
                    r.get("expected_g"),
                    r.get("expected_b"),
                    r.get("tolerance"),
                    r.get("bitmap_name"),
                    r.get("on_match_step_id"),
                    r.get("on_no_match_step_id"),
                ),
            )
        })
        .collect();

    let step_target_map: std::collections::HashMap<i64, String> = if !branch_map.is_empty() {
        let mut map = std::collections::HashMap::with_capacity(raw_steps.len());
        for (idx, row) in raw_steps.iter().enumerate() {
            let step_id: i64 = row.get("id");
            let step_num = idx + 1;
            let step_type: String = row.get("step_type");
            let label: Option<String> = row.get("label");

            let target_str = match label {
                Some(lbl) if !lbl.trim().is_empty() => {
                    format!("Step {} ({})", step_num, lbl.trim())
                }
                _ => format!("Step {} ({})", step_num, step_type),
            };
            map.insert(step_id, target_str);
        }
        map
    } else {
        std::collections::HashMap::new()
    };

    let mut steps = Vec::with_capacity(raw_steps.len());
    for (idx, row) in raw_steps.into_iter().enumerate() {
        let step_id: i64 = row.get("id");
        let step_type: String = row.get("step_type");
        let label: Option<String> = row.get("label");
        let post_delay_ms: i32 = row.get("post_delay_ms");
        let position: f64 = row.get("position");

        let description = match step_type.as_str() {
            "key_press" => key_presses_map
                .get(&step_id)
                .map(|combo| format!("Press {}", combo))
                .unwrap_or_else(|| "Press key".to_string()),
            "mouse_click" => mouse_clicks_map
                .get(&step_id)
                .map(
                    |(x, y, x_var_name, y_var_name, region_var_name, button, click_type)| {
                        generate_mouse_click_summary(
                            *x,
                            *y,
                            x_var_name.as_deref(),
                            y_var_name.as_deref(),
                            region_var_name.as_deref(),
                            button,
                            click_type,
                        )
                    },
                )
                .unwrap_or_else(|| "Click mouse".to_string()),
            "find_pixel_rgb" => find_pixel_map
                .get(&step_id)
                .map(|(x, y, var_name)| {
                    generate_find_pixel_rgb_summary(*x, *y, var_name.as_deref())
                })
                .unwrap_or_else(|| "Read pixel at coordinate".to_string()),
            "find_bitmap" => find_bitmap_map
                .get(&step_id)
                .map(|(bitmap_name, var_name)| {
                    generate_find_bitmap_summary(bitmap_name, var_name.as_deref())
                })
                .unwrap_or_else(|| "Search for bitmap on screen".to_string()),
            "branch" => branch_map
                .get(&step_id)
                .map(
                    |(
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
                    )| {
                        let match_target_str = match_id
                            .and_then(|m_id| step_target_map.get(&m_id))
                            .map(|s| s.as_str());

                        let no_match_target_str = no_match_id
                            .and_then(|n_id| step_target_map.get(&n_id))
                            .map(|s| s.as_str());

                        generate_branch_summary(
                            condition_type,
                            *x,
                            *y,
                            *expected_r,
                            *expected_g,
                            *expected_b,
                            *tolerance,
                            bitmap_name.as_deref(),
                            match_target_str,
                            no_match_target_str,
                        )
                    },
                )
                .unwrap_or_else(|| "BRANCH evaluation".to_string()),
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

// ============================================================================
// GET Handlers for New Step Creation Forms
// ============================================================================

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
    let (steps, bitmaps) = tokio::join!(
        fetch_automation_step_options(&state.db, id, None),
        fetch_available_bitmaps(&state.db, id)
    );

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
    let (bitmaps, variables) = tokio::join!(
        fetch_available_bitmaps(&state.db, id),
        fetch_automation_variables(&state.db, id)
    );

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
        output_region_variable_id: None,
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
        click_mode: "coordinates".to_string(),
        region_variable_id: None,
        button: "left".to_string(),
        click_type: "single".to_string(),
        variables,
        error: None,
        is_edit: false,
    })
}

// ============================================================================
// Helpers for Step Persistence & Error Response Rendering
// ============================================================================

async fn save_base_step(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    automation_id: i64,
    step_id: Option<i64>,
    step_type: &str,
    label: Option<&str>,
    post_delay_seconds: Option<f64>,
) -> Result<i64, sqlx::Error> {
    match step_id {
        Some(sid) => {
            update_base_step(tx, automation_id, sid, label, post_delay_seconds).await?;
            Ok(sid)
        }
        None => insert_base_step(tx, automation_id, step_type, label, post_delay_seconds).await,
    }
}

async fn save_step_details(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    automation_id: i64,
    step_id: i64,
    step_type: &str,
    form: &MouseClickStepForm,
) -> Result<(), sqlx::Error> {
    match step_type {
        "branch" => {
            let condition_type = form.condition_type.as_deref().unwrap_or("pixel_rgb");
            if condition_type == "pixel_rgb" {
                sqlx::query(
                    r#"
                    INSERT INTO step_branches
                        (step_id, automation_id, condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                         reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                         on_match_step_id, on_no_match_step_id)
                    VALUES ($1, $2, 'pixel_rgb', $3, $4, $5, $6, $7, $8, NULL, NULL, NULL, NULL, NULL, NULL, $9, $10)
                    ON CONFLICT (step_id) DO UPDATE SET
                        condition_type = 'pixel_rgb',
                        x = EXCLUDED.x,
                        y = EXCLUDED.y,
                        expected_r = EXCLUDED.expected_r,
                        expected_g = EXCLUDED.expected_g,
                        expected_b = EXCLUDED.expected_b,
                        tolerance = EXCLUDED.tolerance,
                        reference_bitmap_id = NULL,
                        search_x = NULL,
                        search_y = NULL,
                        search_width = NULL,
                        search_height = NULL,
                        match_threshold = NULL,
                        on_match_step_id = EXCLUDED.on_match_step_id,
                        on_no_match_step_id = EXCLUDED.on_no_match_step_id
                    "#,
                )
                .bind(step_id)
                .bind(automation_id)
                .bind(form.x.unwrap_or(0))
                .bind(form.y.unwrap_or(0))
                .bind(form.expected_r.unwrap_or(0))
                .bind(form.expected_g.unwrap_or(0))
                .bind(form.expected_b.unwrap_or(0))
                .bind(form.tolerance.unwrap_or(0))
                .bind(form.on_match_step_id.filter(|&sid| sid > 0))
                .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
                .execute(&mut **tx)
                .await?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO step_branches
                        (step_id, automation_id, condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                         reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                         on_match_step_id, on_no_match_step_id)
                    VALUES ($1, $2, 'bitmap', NULL, NULL, NULL, NULL, NULL, NULL, $3, $4, $5, $6, $7, $8, $9, $10)
                    ON CONFLICT (step_id) DO UPDATE SET
                        condition_type = 'bitmap',
                        x = NULL,
                        y = NULL,
                        expected_r = NULL,
                        expected_g = NULL,
                        expected_b = NULL,
                        tolerance = NULL,
                        reference_bitmap_id = EXCLUDED.reference_bitmap_id,
                        search_x = EXCLUDED.search_x,
                        search_y = EXCLUDED.search_y,
                        search_width = EXCLUDED.search_width,
                        search_height = EXCLUDED.search_height,
                        match_threshold = EXCLUDED.match_threshold,
                        on_match_step_id = EXCLUDED.on_match_step_id,
                        on_no_match_step_id = EXCLUDED.on_no_match_step_id
                    "#,
                )
                .bind(step_id)
                .bind(automation_id)
                .bind(form.reference_bitmap_id)
                .bind(form.search_x)
                .bind(form.search_y)
                .bind(form.search_width)
                .bind(form.search_height)
                .bind(form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0))
                .bind(form.on_match_step_id.filter(|&sid| sid > 0))
                .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
                .execute(&mut **tx)
                .await?;
            }
        }
        "key_press" => {
            let key_combo = form.key_combo.as_deref().unwrap_or("").trim();
            sqlx::query(
                r#"
                INSERT INTO step_key_presses (step_id, key_combo)
                VALUES ($1, $2)
                ON CONFLICT (step_id) DO UPDATE SET key_combo = EXCLUDED.key_combo
                "#,
            )
            .bind(step_id)
            .bind(key_combo)
            .execute(&mut **tx)
            .await?;
        }
        "mouse_click" => {
            let button = sanitize_mouse_button(form.button.as_deref());
            let click_type = sanitize_click_type(form.click_type.as_deref());
            let is_region_mode = form.click_mode.as_deref() == Some("region")
                || (form.click_mode.is_none() && form.region_variable_id.is_some());

            let (x, y, x_var_id, y_var_id, region_var_id) = if is_region_mode {
                (None, None, None, None, form.region_variable_id)
            } else {
                let x_mode = form.x_mode.as_deref().unwrap_or("fixed");
                let y_mode = form.y_mode.as_deref().unwrap_or("fixed");

                let (x_val, x_vid) = if x_mode == "variable" {
                    (None, form.x_variable_id)
                } else {
                    (form.x, None)
                };

                let (y_val, y_vid) = if y_mode == "variable" {
                    (None, form.y_variable_id)
                } else {
                    (form.y, None)
                };

                (x_val, y_val, x_vid, y_vid, None)
            };

            sqlx::query(
                r#"
                INSERT INTO step_mouse_clicks
                    (step_id, x, y, x_variable_id, y_variable_id, region_variable_id, button, click_type)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (step_id) DO UPDATE SET
                    x = EXCLUDED.x,
                    y = EXCLUDED.y,
                    x_variable_id = EXCLUDED.x_variable_id,
                    y_variable_id = EXCLUDED.y_variable_id,
                    region_variable_id = EXCLUDED.region_variable_id,
                    button = EXCLUDED.button,
                    click_type = EXCLUDED.click_type
                "#,
            )
            .bind(step_id)
            .bind(x)
            .bind(y)
            .bind(x_var_id)
            .bind(y_var_id)
            .bind(region_var_id)
            .bind(button)
            .bind(click_type)
            .execute(&mut **tx)
            .await?;
        }
        "find_pixel_rgb" => {
            sqlx::query(
                r#"
                INSERT INTO step_find_pixel_rgb
                    (step_id, x, y, output_variable_id)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (step_id) DO UPDATE SET
                    x = EXCLUDED.x,
                    y = EXCLUDED.y,
                    output_variable_id = EXCLUDED.output_variable_id
                "#,
            )
            .bind(step_id)
            .bind(form.x.unwrap_or(0))
            .bind(form.y.unwrap_or(0))
            .bind(form.output_variable_id)
            .execute(&mut **tx)
            .await?;
        }
        "find_bitmap" => {
            let ref_bitmap_id = form.reference_bitmap_id.unwrap_or(0);
            sqlx::query(
                r#"
                INSERT INTO step_find_bitmap
                    (step_id, reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                     output_found_variable_id, output_x_variable_id, output_y_variable_id, output_region_variable_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (step_id) DO UPDATE SET
                    reference_bitmap_id = EXCLUDED.reference_bitmap_id,
                    search_x = EXCLUDED.search_x,
                    search_y = EXCLUDED.search_y,
                    search_width = EXCLUDED.search_width,
                    search_height = EXCLUDED.search_height,
                    match_threshold = EXCLUDED.match_threshold,
                    output_found_variable_id = EXCLUDED.output_found_variable_id,
                    output_x_variable_id = EXCLUDED.output_x_variable_id,
                    output_y_variable_id = EXCLUDED.output_y_variable_id,
                    output_region_variable_id = EXCLUDED.output_region_variable_id
                "#,
            )
            .bind(step_id)
            .bind(ref_bitmap_id)
            .bind(form.search_x)
            .bind(form.search_y)
            .bind(form.search_width)
            .bind(form.search_height)
            .bind(form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0))
            .bind(form.output_found_variable_id)
            .bind(form.output_x_variable_id)
            .bind(form.output_y_variable_id)
            .bind(form.output_region_variable_id)
            .execute(&mut **tx)
            .await?;
        }
        _ => {}
    }
    Ok(())
}

struct StepErrorCtx<'a> {
    db: &'a PgPool,
    user: AuthUser,
    csrf_token: String,
    automation_id: i64,
    step_id: Option<i64>,
    step_type: &'a str,
}

async fn render_step_error(
    ctx: StepErrorCtx<'_>,
    form: MouseClickStepForm,
    error: String,
) -> axum::response::Response {
    let is_edit = ctx.step_id.is_some();
    let label = form.label.unwrap_or_default();
    let post_delay_seconds = form.post_delay_seconds.unwrap_or(0.0);

    let html_body = match ctx.step_type {
        "branch" => {
            let condition_type = form
                .condition_type
                .unwrap_or_else(|| "pixel_rgb".to_string());
            let (steps, bitmaps) = tokio::join!(
                fetch_automation_step_options(ctx.db, ctx.automation_id, ctx.step_id),
                fetch_available_bitmaps(ctx.db, ctx.automation_id)
            );
            HtmlTemplate(StepBranchTemplate {
                user: ctx.user,
                csrf_token: ctx.csrf_token,
                automation_id: ctx.automation_id,
                step_id: ctx.step_id,
                label,
                post_delay_seconds,
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
                error: Some(error),
                is_edit,
            })
            .into_response()
        }
        "key_press" => HtmlTemplate(StepKeyPressTemplate {
            user: ctx.user,
            csrf_token: ctx.csrf_token,
            automation_id: ctx.automation_id,
            step_id: ctx.step_id,
            label,
            post_delay_seconds,
            key_combo: form.key_combo.unwrap_or_default(),
            error: Some(error),
            is_edit,
        })
        .into_response(),
        "mouse_click" => {
            let variables = fetch_automation_variables(ctx.db, ctx.automation_id).await;
            HtmlTemplate(StepMouseClickTemplate {
                user: ctx.user,
                csrf_token: ctx.csrf_token,
                automation_id: ctx.automation_id,
                step_id: ctx.step_id,
                label,
                post_delay_seconds,
                x_mode: form.x_mode.unwrap_or_else(|| "fixed".to_string()),
                x: form.x,
                x_variable_id: form.x_variable_id,
                y_mode: form.y_mode.unwrap_or_else(|| "fixed".to_string()),
                y: form.y,
                y_variable_id: form.y_variable_id,
                click_mode: form.click_mode.unwrap_or_else(|| "coordinates".to_string()),
                region_variable_id: form.region_variable_id,
                button: form.button.unwrap_or_else(|| "left".to_string()),
                click_type: form.click_type.unwrap_or_else(|| "single".to_string()),
                variables,
                error: Some(error),
                is_edit,
            })
            .into_response()
        }
        "find_pixel_rgb" => {
            let variables = fetch_automation_variables(ctx.db, ctx.automation_id).await;
            HtmlTemplate(StepFindPixelRgbTemplate {
                user: ctx.user,
                csrf_token: ctx.csrf_token,
                automation_id: ctx.automation_id,
                step_id: ctx.step_id,
                label,
                post_delay_seconds,
                x: form.x,
                y: form.y,
                output_variable_id: form.output_variable_id,
                variables,
                error: Some(error),
                is_edit,
            })
            .into_response()
        }
        "find_bitmap" => {
            let (variables, bitmaps) = tokio::join!(
                fetch_automation_variables(ctx.db, ctx.automation_id),
                fetch_available_bitmaps(ctx.db, ctx.automation_id)
            );
            HtmlTemplate(StepFindBitmapTemplate {
                user: ctx.user,
                csrf_token: ctx.csrf_token,
                automation_id: ctx.automation_id,
                step_id: ctx.step_id,
                label,
                post_delay_seconds,
                reference_bitmap_id: form.reference_bitmap_id,
                search_x: form.search_x,
                search_y: form.search_y,
                search_width: form.search_width,
                search_height: form.search_height,
                match_threshold: form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0),
                output_found_variable_id: form.output_found_variable_id,
                output_x_variable_id: form.output_x_variable_id,
                output_y_variable_id: form.output_y_variable_id,
                output_region_variable_id: form.output_region_variable_id,
                bitmaps,
                variables,
                error: Some(error),
                is_edit,
            })
            .into_response()
        }
        _ => return Redirect::to(&format!("/automations/{}", ctx.automation_id)).into_response(),
    };

    (StatusCode::BAD_REQUEST, html_body).into_response()
}

async fn validate_step_form(
    db: &PgPool,
    automation_id: i64,
    step_type: &str,
    form: &MouseClickStepForm,
) -> Result<(), String> {
    match step_type {
        "branch" => {
            let condition_type = form.condition_type.as_deref().unwrap_or("pixel_rgb");
            if let Some(err) = validate_branch_form(form, condition_type) {
                return Err(err);
            }
        }
        "key_press" => {
            let key_combo = form.key_combo.as_deref().unwrap_or("").trim();
            if key_combo.is_empty() {
                return Err("Key combination cannot be empty.".to_string());
            }
        }
        "mouse_click" => {
            let is_region_mode = form.click_mode.as_deref() == Some("region")
                || (form.click_mode.is_none() && form.region_variable_id.is_some());
            if is_region_mode {
                if form.region_variable_id.is_none() {
                    return Err("Please select a valid Region variable.".to_string());
                }
            } else if let Err(err) = validate_literal_or_reference(
                form.x_mode.as_deref(),
                form.x,
                form.x_variable_id,
                "X coordinate",
            ) {
                return Err(err.to_string());
            } else if let Err(err) = validate_literal_or_reference(
                form.y_mode.as_deref(),
                form.y,
                form.y_variable_id,
                "Y coordinate",
            ) {
                return Err(err.to_string());
            }
        }
        "find_pixel_rgb" => {
            if form.x.is_none() || form.y.is_none() {
                return Err("Please provide valid X and Y coordinates.".to_string());
            }
            if let Some(var_id) = form.output_variable_id {
                let vars = fetch_automation_variables(db, automation_id).await;
                if !vars.iter().any(|v| v.id == var_id) {
                    return Err(
                        "Selected output variable is invalid for this automation.".to_string()
                    );
                }
            }
        }
        "find_bitmap" => {
            let vars = fetch_automation_variables(db, automation_id).await;
            if let Err(err) = validate_find_bitmap_output_vars(form, &vars) {
                return Err(err.to_string());
            }
        }
        _ => return Err("Invalid step type.".to_string()),
    }
    Ok(())
}

async fn process_save_step(
    state: &AppState,
    user: &AuthUser,
    automation_id: i64,
    step_id: Option<i64>,
    step_type: &str,
    form: MouseClickStepForm,
) -> axum::response::Response {
    let csrf_token = user.csrf_token.clone();

    if let Err(err_msg) = validate_step_form(&state.db, automation_id, step_type, &form).await {
        let ctx = StepErrorCtx {
            db: &state.db,
            user: user.clone(),
            csrf_token,
            automation_id,
            step_id,
            step_type,
        };
        return render_step_error(ctx, form, err_msg).await;
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start transaction: {}", e);
            return Redirect::to(&format!("/automations/{}", automation_id)).into_response();
        }
    };

    let sid = match save_base_step(
        &mut tx,
        automation_id,
        step_id,
        step_type,
        form.label.as_deref(),
        form.post_delay_seconds,
    )
    .await
    {
        Ok(sid) => sid,
        Err(e) => {
            tracing::error!("Failed to save base step: {}", e);
            return Redirect::to(&format!("/automations/{}", automation_id)).into_response();
        }
    };

    if let Err(e) = save_step_details(&mut tx, automation_id, sid, step_type, &form).await {
        tracing::error!(
            "Failed to save step details for type '{}': {}",
            step_type,
            e
        );
        return Redirect::to(&format!("/automations/{}", automation_id)).into_response();
    }

    let (action, audit_desc) = if step_id.is_some() {
        (
            "edit_step",
            format!("Updated {} step", step_type.replace('_', " ")),
        )
    } else {
        (
            "create_step",
            format!("Created {} step", step_type.replace('_', " ")),
        )
    };

    commit_step_tx_and_audit(&state.db, tx, user.id, action, sid, serde_json::json!({ "automation_id": automation_id, "step_type": step_type, "description": audit_desc })).await;

    Redirect::to(&format!("/automations/{}", automation_id)).into_response()
}

// ============================================================================
// POST Handlers for Step Creation & Editing
// ============================================================================

pub async fn get_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
    Query(query): Query<NewFindBitmapQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let step_row = match sqlx::query(
        "SELECT step_type, label, post_delay_ms FROM automation_steps WHERE id = $1 AND automation_id = $2 AND deleted_at IS NULL",
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
    let label = step_row
        .get::<Option<String>, _>("label")
        .unwrap_or_default();
    let post_delay_seconds = step_row.get::<i32, _>("post_delay_ms") as f64 / 1000.0;

    match step_type.as_str() {
        "key_press" => {
            let key_combo = sqlx::query_scalar::<_, String>(
                "SELECT key_combo FROM step_key_presses WHERE step_id = $1",
            )
            .bind(sid)
            .fetch_optional(&state.db)
            .await
            .unwrap_or_default()
            .unwrap_or_default();

            HtmlTemplate(StepKeyPressTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                key_combo,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "find_bitmap" => {
            let (fb_row, bitmaps, variables) = tokio::join!(
                sqlx::query("SELECT reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold, output_found_variable_id, output_x_variable_id, output_y_variable_id, output_region_variable_id FROM step_find_bitmap WHERE step_id = $1")
                    .bind(sid).fetch_optional(&state.db),
                fetch_available_bitmaps(&state.db, id),
                fetch_automation_variables(&state.db, id),
            );

            let r = fb_row.ok().flatten();
            let (
                db_ref_id,
                db_sx,
                db_sy,
                db_sw,
                db_sh,
                match_threshold,
                out_f,
                out_x,
                out_y,
                out_r,
            ) = match r {
                Some(r) => (
                    r.get("reference_bitmap_id"),
                    r.get("search_x"),
                    r.get("search_y"),
                    r.get("search_width"),
                    r.get("search_height"),
                    r.get("match_threshold"),
                    r.get("output_found_variable_id"),
                    r.get("output_x_variable_id"),
                    r.get("output_y_variable_id"),
                    r.get("output_region_variable_id"),
                ),
                None => (None, None, None, None, None, 0.95, None, None, None, None),
            };

            HtmlTemplate(StepFindBitmapTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                reference_bitmap_id: query.reference_bitmap_id.or(db_ref_id),
                search_x: query.search_x.or(db_sx),
                search_y: query.search_y.or(db_sy),
                search_width: query.search_width.or(db_sw),
                search_height: query.search_height.or(db_sh),
                match_threshold,
                output_found_variable_id: out_f,
                output_x_variable_id: out_x,
                output_y_variable_id: out_y,
                output_region_variable_id: out_r,
                bitmaps,
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "branch" => {
            let (branch_row, steps, bitmaps) = tokio::join!(
                sqlx::query(r#"
                    SELECT condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                           reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                           on_match_step_id, on_no_match_step_id
                    FROM step_branches WHERE step_id = $1
                "#).bind(sid).fetch_optional(&state.db),
                fetch_automation_step_options(&state.db, id, Some(sid)),
                fetch_available_bitmaps(&state.db, id),
            );

            let r = branch_row.ok().flatten();
            let (
                c_type,
                db_x,
                db_y,
                db_r,
                db_g,
                db_b,
                db_tol,
                db_ref,
                db_sx,
                db_sy,
                db_sw,
                db_sh,
                db_thresh,
                on_m,
                on_nm,
            ) = match r {
                Some(r) => (
                    r.get("condition_type"),
                    r.get("x"),
                    r.get("y"),
                    r.get("expected_r"),
                    r.get("expected_g"),
                    r.get("expected_b"),
                    r.get("tolerance"),
                    r.get("reference_bitmap_id"),
                    r.get("search_x"),
                    r.get("search_y"),
                    r.get("search_width"),
                    r.get("search_height"),
                    r.get("match_threshold"),
                    r.get("on_match_step_id"),
                    r.get("on_no_match_step_id"),
                ),
                None => (
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

            HtmlTemplate(StepBranchTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                condition_type: c_type,
                x: query.search_x.or(db_x),
                y: query.search_y.or(db_y),
                expected_r: db_r,
                expected_g: db_g,
                expected_b: db_b,
                tolerance: db_tol,
                reference_bitmap_id: query.reference_bitmap_id.or(db_ref),
                search_x: query.search_x.or(db_sx),
                search_y: query.search_y.or(db_sy),
                search_width: query.search_width.or(db_sw),
                search_height: query.search_height.or(db_sh),
                match_threshold: db_thresh,
                on_match_step_id: on_m,
                on_no_match_step_id: on_nm,
                steps,
                bitmaps,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "find_pixel_rgb" => {
            let (fp_row, variables) = tokio::join!(
                sqlx::query(
                    "SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1"
                )
                .bind(sid)
                .fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let (x, y, output_variable_id) = match fp_row.ok().flatten() {
                Some(r) => (r.get("x"), r.get("y"), r.get("output_variable_id")),
                None => (None, None, None),
            };

            HtmlTemplate(StepFindPixelRgbTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                x,
                y,
                output_variable_id,
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "mouse_click" => {
            let (mc_row, variables) = tokio::join!(
                sqlx::query("SELECT x, y, x_variable_id, y_variable_id, region_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
                    .bind(sid).fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let (x, y, x_var_id, y_var_id, region_var_id, button, click_type) =
                match mc_row.ok().flatten() {
                    Some(r) => (
                        r.get("x"),
                        r.get("y"),
                        r.get("x_variable_id"),
                        r.get("y_variable_id"),
                        r.get("region_variable_id"),
                        r.get("button"),
                        r.get("click_type"),
                    ),
                    None => (
                        None,
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
            let click_mode = if region_var_id.is_some() {
                "region"
            } else {
                "coordinates"
            }
            .to_string();

            HtmlTemplate(StepMouseClickTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                x_mode,
                x,
                x_variable_id: x_var_id,
                y_mode,
                y,
                y_variable_id: y_var_id,
                click_mode,
                region_variable_id: region_var_id,
                button,
                click_type,
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        _ => Redirect::to(&format!("/automations/{}", id)).into_response(),
    }
}

/// POST /automations/{id}/steps
#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<MouseClickStepForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type = form.step_type.as_deref().unwrap_or("").to_string();
    process_save_step(&state, &user, id, None, &step_type, form).await
}

/// POST /automations/{id}/steps/{sid}
#[tracing::instrument(skip(state, user, form))]
pub async fn post_edit_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, sid)): Path<(i64, i64)>,
    CsrfForm(form): CsrfForm<MouseClickStepForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type_row = sqlx::query(
        "SELECT step_type FROM automation_steps WHERE id = $1 AND automation_id = $2 AND deleted_at IS NULL",
    )
    .bind(sid)
    .bind(id)
    .fetch_optional(&state.db)
    .await;

    let step_type: String = match step_type_row {
        Ok(Some(r)) => r.get("step_type"),
        _ => return Redirect::to(&format!("/automations/{}", id)).into_response(),
    };

    process_save_step(&state, &user, id, Some(sid), &step_type, form).await
}

// ============================================================================
// Reordering & Deletion Handlers
// ============================================================================

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

    let steps = sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC")
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

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_mouse_click_summary_and_region_variable() {
        let summary_region = generate_mouse_click_summary(
            None,
            None,
            None,
            None,
            Some("login_box"),
            "left",
            "single",
        );
        assert_eq!(
            summary_region,
            "Click center of region «login_box» [left, single]"
        );

        let summary_coords =
            generate_mouse_click_summary(Some(100), Some(200), None, None, None, "left", "single");
        assert_eq!(summary_coords, "Click (100, 200) [left, single]");
    }

    #[test]
    fn test_validate_literal_or_reference() {
        let r1 = validate_literal_or_reference(Some("fixed"), Some(824), None, "X coordinate");
        assert_eq!(r1, Ok((Some(824), None)));

        let r2 = validate_literal_or_reference::<i32>(Some("fixed"), None, None, "X coordinate");
        assert!(r2.is_err());
        assert!(
            r2.unwrap_err()
                .contains("Please provide a fixed value for X coordinate.")
        );

        let r3 =
            validate_literal_or_reference::<i32>(Some("variable"), None, Some(10), "X coordinate");
        assert_eq!(r3, Ok((None, Some(10))));

        let r4 =
            validate_literal_or_reference::<i32>(Some("variable"), None, Some(0), "X coordinate");
        assert!(r4.is_err());
        assert!(
            r4.unwrap_err()
                .contains("Please select a valid variable for X coordinate.")
        );

        let r5 = validate_literal_or_reference::<i32>(Some("variable"), None, None, "X coordinate");
        assert!(r5.is_err());

        let r6 = validate_literal_or_reference(None, Some(100), None, "X coordinate");
        assert_eq!(r6, Ok((Some(100), None)));

        let r7 = validate_literal_or_reference::<i32>(None, None, Some(15), "X coordinate");
        assert_eq!(r7, Ok((None, Some(15))));

        let r8 = validate_literal_or_reference(None, Some(100), Some(15), "X coordinate");
        assert!(r8.is_err());
        assert!(
            r8.unwrap_err()
                .contains("Cannot specify both a fixed value and a variable for X coordinate.")
        );

        let r9 = validate_literal_or_reference::<i32>(None, None, None, "X coordinate");
        assert!(r9.is_err());
        assert!(
            r9.unwrap_err()
                .contains("Please specify either a fixed value or a variable for X coordinate.")
        );
    }

    #[test]
    fn test_generate_mouse_click_summary() {
        let s1 =
            generate_mouse_click_summary(Some(824), Some(391), None, None, None, "left", "single");
        assert_eq!(s1, "Click (824, 391) [left, single]");

        let s2 = generate_mouse_click_summary(
            None,
            Some(391),
            Some("target_x"),
            None,
            None,
            "right",
            "double",
        );
        assert_eq!(s2, "Click («target_x», 391) [right, double]");

        let s3 = generate_mouse_click_summary(
            None,
            None,
            Some("target_x"),
            Some("target_y"),
            None,
            "middle",
            "single",
        );
        assert_eq!(s3, "Click («target_x», «target_y») [middle, single]");

        let s4 = generate_mouse_click_summary(None, Some(100), None, None, None, "left", "single");
        assert_eq!(s4, "Click (variable, 100) [left, single]");
    }

    #[test]
    fn test_generate_find_pixel_rgb_summary() {
        let s1 = generate_find_pixel_rgb_summary(100, 200, None);
        assert_eq!(s1, "Read pixel at (100, 200)");

        let s2 = generate_find_pixel_rgb_summary(100, 200, Some("bg_color"));
        assert_eq!(s2, "Read pixel at (100, 200) → store as «bg_color»");

        let s3 = generate_find_pixel_rgb_summary(150, 250, Some("   "));
        assert_eq!(s3, "Read pixel at (150, 250)");
    }

    #[test]
    fn test_generate_find_bitmap_summary() {
        let s1 = generate_find_bitmap_summary("login_button", None);
        assert_eq!(s1, "Search for «login_button» on screen");

        let s2 = generate_find_bitmap_summary("login_button", Some("found_login"));
        assert_eq!(
            s2,
            "Search for «login_button» on screen → store as «found_login»"
        );

        let s3 = generate_find_bitmap_summary("login_button", Some("   "));
        assert_eq!(s3, "Search for «login_button» on screen");
    }

    #[test]
    fn test_generate_branch_summary() {
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
            click_mode: "coordinates".to_string(),
            region_variable_id: None,
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
            click_mode: "coordinates".to_string(),
            region_variable_id: None,
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
            output_region_variable_id: None,
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
        let summary1 = generate_mouse_click_summary(
            None,
            None,
            Some("button_x"),
            Some("button_y"),
            None,
            "left",
            "single",
        );
        assert_eq!(summary1, "Click («button_x», «button_y») [left, single]");

        let summary2 = generate_mouse_click_summary(
            None,
            Some(250),
            Some("target_x"),
            None,
            None,
            "right",
            "double",
        );
        assert_eq!(summary2, "Click («target_x», 250) [right, double]");

        let summary3 = generate_mouse_click_summary(
            Some(500),
            None,
            None,
            Some("target_y"),
            None,
            "middle",
            "single",
        );
        assert_eq!(summary3, "Click (500, «target_y») [middle, single]");
    }
}
