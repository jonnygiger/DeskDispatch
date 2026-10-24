use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use sqlx::PgPool;

use super::types::*;
use super::variables::fetch_automation_variables;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, log_audit};
use crate::routes::auth::HtmlTemplate;

// ============================================================================
// Strongly-Typed Row Structs for Database Queries
// ============================================================================

#[derive(sqlx::FromRow)]
struct RawStepOptionRow {
    id: i64,
    step_type: String,
    label: Option<String>,
    position: f64,
}

#[derive(sqlx::FromRow)]
struct MaxPosRow {
    max_pos: Option<f64>,
}

#[derive(sqlx::FromRow)]
struct StepIdRow {
    id: i64,
}

#[derive(sqlx::FromRow)]
struct StepTypeRow {
    step_type: String,
}

#[derive(sqlx::FromRow)]
struct StepPosRow {
    id: i64,
    position: f64,
}

#[derive(sqlx::FromRow)]
struct BaseStepRow {
    step_type: String,
    label: Option<String>,
    post_delay_ms: i32,
}

#[derive(sqlx::FromRow)]
struct FindBitmapDetailRow {
    reference_bitmap_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_width: Option<i32>,
    search_height: Option<i32>,
    match_threshold: f32,
    output_found_variable_id: Option<i64>,
    output_x_variable_id: Option<i64>,
    output_y_variable_id: Option<i64>,
    output_region_variable_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct BranchDetailRow {
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
    window_match_pattern: Option<String>,
    #[allow(dead_code)]
    window_match_type: Option<String>,
    window_check_type: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FindPixelDetailRow {
    x: Option<i32>,
    y: Option<i32>,
    output_variable_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct MouseScrollDetailRow {
    direction: String,
    scroll_amount: i32,
    x: Option<i32>,
    y: Option<i32>,
    x_variable_id: Option<i64>,
    y_variable_id: Option<i64>,
    region_variable_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct MouseClickDetailRow {
    x: Option<i32>,
    y: Option<i32>,
    x_variable_id: Option<i64>,
    y_variable_id: Option<i64>,
    region_variable_id: Option<i64>,
    button: String,
    click_type: String,
}

#[derive(sqlx::FromRow)]
struct OcrExtractDetailRow {
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_width: Option<i32>,
    search_height: Option<i32>,
    region_variable_id: Option<i64>,
    output_variable_id: Option<i64>,
    set_as_task_output: bool,
}

#[derive(sqlx::FromRow)]
struct TypeTextDetailRow {
    text_to_type: String,
    delay_between_keys_ms: i32,
}

#[derive(sqlx::FromRow)]
struct WindowControlViewRow {
    step_id: i64,
    action: String,
    match_pattern: String,
    match_type: String,
    timeout_ms: i64,
}

#[derive(sqlx::FromRow)]
struct WindowControlDetailRow {
    action: String,
    match_pattern: String,
    match_type: String,
    timeout_ms: i64,
}

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

fn validate_branch_form(form: &StepForm, condition_type: &str) -> Option<String> {
    if form.on_match_step_id.filter(|&sid| sid > 0).is_none()
        || form.on_no_match_step_id.filter(|&sid| sid > 0).is_none()
    {
        Some("Both match and no-match target steps are required for branch conditions.".to_string())
    } else if condition_type == "pixel_rgb" && (form.x.is_none() || form.y.is_none()) {
        Some("Please provide valid X and Y coordinates for pixel RGB condition.".to_string())
    } else if condition_type == "bitmap" && form.reference_bitmap_id.filter(|&id| id > 0).is_none()
    {
        Some("Please select a reference bitmap for bitmap condition.".to_string())
    } else if condition_type == "window_state"
        && form
            .window_match_pattern
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        Some("Please provide a window title match pattern for window state condition.".to_string())
    } else {
        None
    }
}

#[inline]
pub fn sanitize_label(label: Option<&str>) -> Option<&str> {
    label.map(str::trim).filter(|s| !s.is_empty())
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
    sqlx::query_as::<_, MaxPosRow>(
        "SELECT MAX(position) AS max_pos FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(executor)
    .await
    .ok()
    .and_then(|r| r.max_pos)
    .map(|p| p + 10.0)
    .unwrap_or(10.0)
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

    let row = sqlx::query_as::<_, StepIdRow>(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(automation_id)
    .bind(next_pos)
    .bind(step_type)
    .bind(clean_label)
    .bind(post_delay_ms)
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.id)
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
    .bind(clean_label)
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
    let raw_steps = sqlx::query_as::<_, RawStepOptionRow>(
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
            if exclude_step_id == Some(row.id) {
                return None;
            }
            Some(StepOption {
                id: row.id,
                step_type: row.step_type,
                label: row.label,
                position: row.position,
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

pub struct StepOptionsBundle {
    pub variables: Vec<VariableOption>,
    pub bitmaps: Vec<BitmapOption>,
    pub steps: Vec<StepOption>,
}

pub async fn fetch_step_options_bundle(
    db: &PgPool,
    automation_id: i64,
    exclude_step_id: Option<i64>,
) -> StepOptionsBundle {
    let (variables, bitmaps, steps) = tokio::join!(
        fetch_automation_variables(db, automation_id),
        fetch_available_bitmaps(db, automation_id),
        fetch_automation_step_options(db, automation_id, exclude_step_id)
    );
    StepOptionsBundle {
        variables,
        bitmaps,
        steps,
    }
}

pub fn generate_mouse_scroll_summary(
    direction: &str,
    scroll_amount: i32,
    x: Option<i32>,
    y: Option<i32>,
    x_var_name: Option<&str>,
    y_var_name: Option<&str>,
    region_var_name: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let _ = write!(buf, "Scroll {direction} ({scroll_amount} ticks)");

    if let Some(rname) = region_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(buf, " at region «{rname}» center");
        return buf;
    }

    if x.is_some() || y.is_some() || x_var_name.is_some() || y_var_name.is_some() {
        buf.push_str(" at (");
        if let Some(name) = x_var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, "«{name}»");
        } else if let Some(val) = x {
            let _ = write!(buf, "{val}");
        } else {
            buf.push('0');
        }

        buf.push_str(", ");
        if let Some(name) = y_var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, "«{name}»");
        } else if let Some(val) = y {
            let _ = write!(buf, "{val}");
        } else {
            buf.push('0');
        }

        buf.push(')');
    }

    buf
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
    let mut buf = String::with_capacity(64);
    if let Some(rname) = region_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(
            buf,
            "Click center of region «{rname}» [{button}, {click_type}]"
        );
        return buf;
    }

    buf.push_str("Click (");
    if let Some(name) = x_var_name.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = write!(buf, "«{name}»");
    } else if let Some(val) = x {
        let _ = write!(buf, "{val}");
    } else {
        buf.push_str("variable");
    }

    buf.push_str(", ");
    if let Some(name) = y_var_name.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = write!(buf, "«{name}»");
    } else if let Some(val) = y {
        let _ = write!(buf, "{val}");
    } else {
        buf.push_str("variable");
    }

    let _ = write!(buf, ") [{button}, {click_type}]");
    buf
}

pub fn generate_type_text_summary(text_to_type: &str, delay_between_keys_ms: i32) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let trimmed = text_to_type.trim();

    if delay_between_keys_ms > 0 {
        if trimmed.len() > 30 {
            let _ = write!(
                buf,
                "Type \"{}...\" ({}ms key delay)",
                &trimmed[..30],
                delay_between_keys_ms
            );
        } else {
            let _ = write!(
                buf,
                "Type \"{}\" ({}ms key delay)",
                trimmed, delay_between_keys_ms
            );
        }
    } else {
        if trimmed.len() > 30 {
            let _ = write!(buf, "Type \"{}...\"", &trimmed[..30]);
        } else {
            let _ = write!(buf, "Type \"{}\"", trimmed);
        }
    }
    buf
}

pub fn generate_ocr_extract_summary(
    search_x: Option<i32>,
    search_y: Option<i32>,
    search_width: Option<i32>,
    search_height: Option<i32>,
    region_var_name: Option<&str>,
    output_var_name: Option<&str>,
    set_as_task_output: bool,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(96);
    buf.push_str("OCR Extract from ");

    if let Some(rname) = region_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(buf, "region «{rname}»");
    } else if let (Some(x), Some(y), Some(w), Some(h)) =
        (search_x, search_y, search_width, search_height)
    {
        let _ = write!(buf, "region ({x}, {y}, {w}x{h})");
    } else {
        buf.push_str("full screen");
    }

    if let Some(oname) = output_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(buf, " → store in «{oname}»");
    }

    if set_as_task_output {
        buf.push_str(" [task output]");
    }

    buf
}

pub fn generate_window_control_summary(
    action: &str,
    match_pattern: &str,
    match_type: &str,
    timeout_ms: i64,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(96);
    let action_str = if action.eq_ignore_ascii_case("activate") {
        "Focus"
    } else if action.eq_ignore_ascii_case("close") {
        "Close"
    } else if action.eq_ignore_ascii_case("minimize") {
        "Minimize"
    } else if action.eq_ignore_ascii_case("maximize") {
        "Maximize"
    } else if action.eq_ignore_ascii_case("wait") {
        "Wait for"
    } else {
        "Control"
    };

    let pattern_trimmed = match_pattern.trim();
    let display_idx = pattern_trimmed
        .char_indices()
        .nth(27)
        .map(|(i, _)| i)
        .unwrap_or(pattern_trimmed.len());
    let (display_pattern, suffix) = if display_idx < pattern_trimmed.len() {
        (&pattern_trimmed[..display_idx], "...")
    } else {
        (pattern_trimmed, "")
    };

    if action.eq_ignore_ascii_case("wait") {
        let _ = write!(
            buf,
            "Wait for window matching \"{display_pattern}{suffix}\" ({match_type}) [timeout {:.1}s]",
            timeout_ms as f64 / 1000.0
        );
    } else {
        let _ = write!(
            buf,
            "{action_str} window matching \"{display_pattern}{suffix}\" ({match_type})"
        );
    }
    buf
}

pub fn generate_clipboard_control_summary(
    action: &str,
    text_to_set: Option<&str>,
    input_var_name: Option<&str>,
    output_var_name: Option<&str>,
    set_as_task_output: bool,
    timeout_ms: i64,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(96);
    if action.eq_ignore_ascii_case("set_text") {
        let _ = write!(buf, "Set clipboard to ");
        if let Some(var_name) = input_var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, "«{}»", var_name);
        } else if let Some(txt) = text_to_set.map(str::trim).filter(|s| !s.is_empty()) {
            if txt.chars().count() > 30 {
                let _ = write!(buf, "\"{}\"...", txt.chars().take(27).collect::<String>());
            } else {
                let _ = write!(buf, "\"{}\"", txt);
            }
        } else {
            let _ = write!(buf, "text");
        }
    } else if action.eq_ignore_ascii_case("get_text") {
        let _ = write!(buf, "Read clipboard text");
        if let Some(var_name) = output_var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, " → store in «{}»", var_name);
        }
        if set_as_task_output {
            let _ = write!(buf, " (set as task output)");
        }
    } else if action.eq_ignore_ascii_case("clear") {
        let _ = write!(buf, "Clear clipboard");
    } else if action.eq_ignore_ascii_case("wait_for_change") {
        let _ = write!(
            buf,
            "Wait for clipboard change [timeout {:.1}s]",
            timeout_ms as f64 / 1000.0
        );
        if let Some(var_name) = output_var_name.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = write!(buf, " → store in «{}»", var_name);
        }
        if set_as_task_output {
            let _ = write!(buf, " (set as task output)");
        }
    } else {
        let _ = write!(buf, "Clipboard operation");
    }
    buf
}

pub fn generate_find_pixel_rgb_summary(x: i32, y: i32, output_var_name: Option<&str>) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let _ = write!(buf, "Read pixel at ({x}, {y})");
    if let Some(name) = output_var_name.map(str::trim).filter(|n| !n.is_empty()) {
        let _ = write!(buf, " → store as «{name}»");
    }
    buf
}

pub fn generate_find_bitmap_summary(
    bitmap_name: &str,
    output_found_var_name: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(64);
    let _ = write!(buf, "Search for «{bitmap_name}» on screen");
    if let Some(name) = output_found_var_name
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let _ = write!(buf, " → store as «{name}»");
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
    window_pattern: Option<&str>,
    window_check_type: Option<&str>,
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
            "BRANCH: if pixel at ({}, {}) ≈ RGB({},{},{}) ±{} → go to {match_str}, else → go to {no_match_str}",
            x.unwrap_or(0),
            y.unwrap_or(0),
            expected_r.unwrap_or(0),
            expected_g.unwrap_or(0),
            expected_b.unwrap_or(0),
            tolerance.unwrap_or(0),
        );
    } else if condition_type == "window_state" {
        let pattern = window_pattern.unwrap_or("window");
        let check_state = if window_check_type == Some("active") {
            "active"
        } else {
            "exists"
        };
        let _ = write!(
            buf,
            "BRANCH: if window \"{pattern}\" {check_state} → go to {match_str}, else → go to {no_match_str}"
        );
    } else {
        let bm_name = bitmap_name.unwrap_or("bitmap");
        let _ = write!(
            buf,
            "BRANCH: if bitmap «{bm_name}» found → go to {match_str}, else → go to {no_match_str}"
        );
    }
    buf
}

#[derive(sqlx::FromRow)]
struct RawStepRow {
    id: i64,
    step_type: String,
    label: Option<String>,
    post_delay_ms: i32,
    position: f64,
}

#[derive(sqlx::FromRow)]
struct KeyPressViewRow {
    step_id: i64,
    key_combo: String,
}

#[derive(sqlx::FromRow)]
struct TypeTextViewRow {
    step_id: i64,
    text_to_type: String,
    delay_between_keys_ms: i32,
}

#[derive(sqlx::FromRow)]
struct MouseScrollViewRow {
    step_id: i64,
    direction: String,
    scroll_amount: i32,
    x: Option<i32>,
    y: Option<i32>,
    x_var_name: Option<String>,
    y_var_name: Option<String>,
    region_var_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct MouseClickViewRow {
    step_id: i64,
    x: Option<i32>,
    y: Option<i32>,
    x_var_name: Option<String>,
    y_var_name: Option<String>,
    region_var_name: Option<String>,
    button: String,
    click_type: String,
}

#[derive(sqlx::FromRow)]
struct FindPixelViewRow {
    step_id: i64,
    x: i32,
    y: i32,
    var_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FindBitmapViewRow {
    step_id: i64,
    reference_bitmap_id: Option<i64>,
    reference_bitmap_key: Option<String>,
    bitmap_name: String,
    var_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BranchViewRow {
    step_id: i64,
    condition_type: String,
    x: Option<i32>,
    y: Option<i32>,
    expected_r: Option<i16>,
    expected_g: Option<i16>,
    expected_b: Option<i16>,
    tolerance: Option<i16>,
    reference_bitmap_id: Option<i64>,
    reference_bitmap_key: Option<String>,
    bitmap_name: Option<String>,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
    window_match_pattern: Option<String>,
    #[allow(dead_code)]
    window_match_type: Option<String>,
    window_check_type: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StepScreenshotViewRow {
    step_id: i64,
    object_storage_key: String,
}

/// Bulk fetch and format all step view items for an automation.
/// Optimization: Bulk-loads steps and all step subtype tables concurrently in O(1) database
/// queries using `tokio::join!` with strongly-typed `FromRow` structs, eliminating runtime column lookups.
pub async fn fetch_automation_step_views(db: &PgPool, automation_id: i64) -> Vec<StepViewItem> {
    let raw_steps = match sqlx::query_as::<_, RawStepRow>(
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

    let (kp_res, ms_res, wc_res, tt_res, mc_res, fp_res, fb_res, br_res, ss_res) = tokio::join!(
        sqlx::query_as::<_, KeyPressViewRow>(
            r#"
            SELECT skp.step_id, skp.key_combo
            FROM step_key_presses skp
            JOIN automation_steps s ON skp.step_id = s.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, MouseScrollViewRow>(
            r#"
            SELECT ms.step_id, ms.direction, ms.scroll_amount, ms.x, ms.y, vx.name AS x_var_name, vy.name AS y_var_name, vr.name AS region_var_name
            FROM step_mouse_scrolls ms
            JOIN automation_steps s ON ms.step_id = s.id
            LEFT JOIN automation_variables vx ON ms.x_variable_id = vx.id
            LEFT JOIN automation_variables vy ON ms.y_variable_id = vy.id
            LEFT JOIN automation_variables vr ON ms.region_variable_id = vr.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, WindowControlViewRow>(
            r#"
            SELECT swc.step_id, swc.action, swc.match_pattern, swc.match_type, swc.timeout_ms
            FROM step_window_controls swc
            JOIN automation_steps s ON swc.step_id = s.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, TypeTextViewRow>(
            r#"
            SELECT stt.step_id, stt.text_to_type, stt.delay_between_keys_ms
            FROM step_type_texts stt
            JOIN automation_steps s ON stt.step_id = s.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, MouseClickViewRow>(
            r#"
            SELECT mc.step_id, mc.x, mc.y, mc.button, mc.click_type, vx.name AS x_var_name, vy.name AS y_var_name, vr.name AS region_var_name
            FROM step_mouse_clicks mc
            JOIN automation_steps s ON mc.step_id = s.id
            LEFT JOIN automation_variables vx ON mc.x_variable_id = vx.id
            LEFT JOIN automation_variables vy ON mc.y_variable_id = vy.id
            LEFT JOIN automation_variables vr ON mc.region_variable_id = vr.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, FindPixelViewRow>(
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

        sqlx::query_as::<_, FindBitmapViewRow>(
            r#"
            SELECT fb.step_id, fb.reference_bitmap_id, b.name AS bitmap_name, b.object_storage_key AS reference_bitmap_key, v.name AS var_name
            FROM step_find_bitmap fb
            JOIN automation_steps s ON fb.step_id = s.id
            JOIN bitmaps b ON fb.reference_bitmap_id = b.id
            LEFT JOIN automation_variables v ON fb.output_found_variable_id = v.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, BranchViewRow>(
            r#"
            SELECT
                sb.step_id, sb.condition_type, sb.x, sb.y, sb.expected_r, sb.expected_g, sb.expected_b, sb.tolerance,
                sb.reference_bitmap_id, b.name AS bitmap_name, b.object_storage_key AS reference_bitmap_key, sb.on_match_step_id, sb.on_no_match_step_id,
                sb.window_match_pattern, sb.window_match_type, sb.window_check_type
            FROM step_branches sb
            JOIN automation_steps s ON sb.step_id = s.id
            LEFT JOIN bitmaps b ON sb.reference_bitmap_id = b.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),

        sqlx::query_as::<_, StepScreenshotViewRow>(
            r#"
            SELECT ss.step_id, ss.object_storage_key
            FROM step_screenshots ss
            JOIN automation_steps s ON ss.step_id = s.id
            WHERE s.automation_id = $1 AND s.deleted_at IS NULL
            "#,
        )
        .bind(automation_id)
        .fetch_all(db),
    );

    let kp_rows = kp_res.unwrap_or_default();
    let wc_rows = wc_res.unwrap_or_default();
    let tt_rows = tt_res.unwrap_or_default();
    let mc_rows = mc_res.unwrap_or_default();
    let ms_rows = ms_res.unwrap_or_default();
    let fp_rows = fp_res.unwrap_or_default();
    let fb_rows = fb_res.unwrap_or_default();
    let br_rows = br_res.unwrap_or_default();
    let ss_rows = ss_res.unwrap_or_default();

    enum StepDetailView {
        KeyPress(KeyPressViewRow),
        TypeText(TypeTextViewRow),
        WindowControl(WindowControlViewRow),
        MouseClick(MouseClickViewRow),
        MouseScroll(MouseScrollViewRow),
        FindPixel(FindPixelViewRow),
        FindBitmap(FindBitmapViewRow),
        Branch(BranchViewRow),
    }

    use std::collections::HashMap;

    let step_target_map = if !br_rows.is_empty() {
        let target_ids: std::collections::HashSet<i64> = br_rows
            .iter()
            .flat_map(|r| [r.on_match_step_id, r.on_no_match_step_id])
            .flatten()
            .collect();

        let mut target_map = HashMap::with_capacity(target_ids.len());
        for (idx, row) in raw_steps.iter().enumerate() {
            if target_ids.contains(&row.id) {
                let step_num = idx + 1;
                let target_str = match &row.label {
                    Some(lbl) if !lbl.trim().is_empty() => {
                        format!("Step {step_num} ({})", lbl.trim())
                    }
                    _ => format!("Step {step_num} ({})", row.step_type),
                };
                target_map.insert(row.id, target_str);
            }
        }
        target_map
    } else {
        HashMap::new()
    };

    let mut details_map: HashMap<i64, StepDetailView> = HashMap::with_capacity(raw_steps.len());

    for r in kp_rows {
        details_map.insert(r.step_id, StepDetailView::KeyPress(r));
    }
    for r in wc_rows {
        details_map.insert(r.step_id, StepDetailView::WindowControl(r));
    }
    for r in tt_rows {
        details_map.insert(r.step_id, StepDetailView::TypeText(r));
    }
    for r in mc_rows {
        details_map.insert(r.step_id, StepDetailView::MouseClick(r));
    }
    for r in ms_rows {
        details_map.insert(r.step_id, StepDetailView::MouseScroll(r));
    }
    for r in fp_rows {
        details_map.insert(r.step_id, StepDetailView::FindPixel(r));
    }
    for r in fb_rows {
        details_map.insert(r.step_id, StepDetailView::FindBitmap(r));
    }
    for r in br_rows {
        details_map.insert(r.step_id, StepDetailView::Branch(r));
    }

    let mut step_screenshot_map: HashMap<i64, String> = HashMap::with_capacity(ss_rows.len());
    for r in ss_rows {
        step_screenshot_map.insert(r.step_id, r.object_storage_key);
    }

    let mut steps = Vec::with_capacity(raw_steps.len());
    for (idx, row) in raw_steps.into_iter().enumerate() {
        let step_id = row.id;
        let step_type = row.step_type;
        let label = row.label;
        let post_delay_ms = row.post_delay_ms;
        let position = row.position;

        let detail_opt = details_map.remove(&step_id);

        let (description, ref_bitmap_id, ref_bitmap_key) = match detail_opt {
            Some(StepDetailView::KeyPress(r)) => (format!("Press {}", r.key_combo), None, None),
            Some(StepDetailView::TypeText(r)) => (
                generate_type_text_summary(&r.text_to_type, r.delay_between_keys_ms),
                None,
                None,
            ),
            Some(StepDetailView::MouseScroll(r)) => (
                generate_mouse_scroll_summary(
                    &r.direction,
                    r.scroll_amount,
                    r.x,
                    r.y,
                    r.x_var_name.as_deref(),
                    r.y_var_name.as_deref(),
                    r.region_var_name.as_deref(),
                ),
                None,
                None,
            ),
            Some(StepDetailView::WindowControl(r)) => (
                generate_window_control_summary(
                    &r.action,
                    &r.match_pattern,
                    &r.match_type,
                    r.timeout_ms,
                ),
                None,
                None,
            ),
            Some(StepDetailView::MouseClick(r)) => (
                generate_mouse_click_summary(
                    r.x,
                    r.y,
                    r.x_var_name.as_deref(),
                    r.y_var_name.as_deref(),
                    r.region_var_name.as_deref(),
                    &r.button,
                    &r.click_type,
                ),
                None,
                None,
            ),
            Some(StepDetailView::FindPixel(r)) => (
                generate_find_pixel_rgb_summary(r.x, r.y, r.var_name.as_deref()),
                None,
                None,
            ),
            Some(StepDetailView::FindBitmap(r)) => (
                generate_find_bitmap_summary(&r.bitmap_name, r.var_name.as_deref()),
                r.reference_bitmap_id,
                r.reference_bitmap_key,
            ),
            Some(StepDetailView::Branch(r)) => {
                let match_target_str = r
                    .on_match_step_id
                    .and_then(|m_id| step_target_map.get(&m_id))
                    .map(String::as_str);

                let no_match_target_str = r
                    .on_no_match_step_id
                    .and_then(|n_id| step_target_map.get(&n_id))
                    .map(String::as_str);

                let summary = generate_branch_summary(
                    &r.condition_type,
                    r.x,
                    r.y,
                    r.expected_r,
                    r.expected_g,
                    r.expected_b,
                    r.tolerance,
                    r.bitmap_name.as_deref(),
                    r.window_match_pattern.as_deref(),
                    r.window_check_type.as_deref(),
                    match_target_str,
                    no_match_target_str,
                );
                (summary, r.reference_bitmap_id, r.reference_bitmap_key)
            }
            None => (
                match step_type.as_str() {
                    "key_press" => "Press key".to_string(),
                    "type_text" => "Type text".to_string(),
                    "window_control" => "Control window".to_string(),
                    "mouse_click" => "Click mouse".to_string(),
                    "mouse_scroll" => "Mouse scroll".to_string(),
                    "find_pixel_rgb" => "Read pixel at coordinate".to_string(),
                    "find_bitmap" => "Search for bitmap on screen".to_string(),
                    "branch" => "BRANCH evaluation".to_string(),
                    _ => "Unknown step".to_string(),
                },
                None,
                None,
            ),
        };

        let screenshot_key = step_screenshot_map.remove(&step_id);

        steps.push(StepViewItem {
            id: step_id,
            step_number: idx + 1,
            step_type,
            label,
            post_delay_ms,
            position,
            description,
            reference_bitmap_id: ref_bitmap_id,
            reference_bitmap_key: ref_bitmap_key,
            screenshot_key,
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

/// GET /automations/{id}/steps/new/{step_type}
#[tracing::instrument(skip(state, user))]
pub async fn get_new_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, step_type)): Path<(i64, String)>,
    Query(query): Query<EditStepQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let form = StepForm {
        reference_bitmap_id: query.reference_bitmap_id,
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
        condition_type: query.condition_type,
        x: query.x,
        y: query.y,
        ..Default::default()
    };
    render_step_form(
        &state.db, user, csrf_token, id, None, &step_type, form, None,
    )
    .await
}

pub async fn get_new_key_press_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    get_new_step_handler(
        State(state),
        user,
        Path((id, "key_press".to_string())),
        Query(EditStepQuery::default()),
    )
    .await
}

pub async fn get_new_ocr_extract_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewOcrExtractQuery>,
) -> impl IntoResponse {
    let edit_query = EditStepQuery {
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
        ..Default::default()
    };
    get_new_step_handler(
        State(state),
        user,
        Path((id, "ocr_extract".to_string())),
        Query(edit_query),
    )
    .await
}

pub async fn get_new_window_control_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    get_new_step_handler(
        State(state),
        user,
        Path((id, "window_control".to_string())),
        Query(EditStepQuery::default()),
    )
    .await
}

pub async fn get_new_type_text_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    get_new_step_handler(
        State(state),
        user,
        Path((id, "type_text".to_string())),
        Query(EditStepQuery::default()),
    )
    .await
}

pub async fn get_new_branch_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewBranchQuery>,
) -> impl IntoResponse {
    let edit_query = EditStepQuery {
        condition_type: query.condition_type,
        x: query.x,
        y: query.y,
        reference_bitmap_id: query.reference_bitmap_id,
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
    };
    get_new_step_handler(
        State(state),
        user,
        Path((id, "branch".to_string())),
        Query(edit_query),
    )
    .await
}

pub async fn get_new_find_bitmap_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewFindBitmapQuery>,
) -> impl IntoResponse {
    let edit_query = EditStepQuery {
        reference_bitmap_id: query.reference_bitmap_id,
        search_x: query.search_x,
        search_y: query.search_y,
        search_width: query.search_width,
        search_height: query.search_height,
        ..Default::default()
    };
    get_new_step_handler(
        State(state),
        user,
        Path((id, "find_bitmap".to_string())),
        Query(edit_query),
    )
    .await
}

pub async fn get_new_find_pixel_rgb_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewFindPixelRgbQuery>,
) -> impl IntoResponse {
    let edit_query = EditStepQuery {
        x: query.x,
        y: query.y,
        ..Default::default()
    };
    get_new_step_handler(
        State(state),
        user,
        Path((id, "find_pixel_rgb".to_string())),
        Query(edit_query),
    )
    .await
}

pub async fn get_new_mouse_scroll_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    get_new_step_handler(
        State(state),
        user,
        Path((id, "mouse_scroll".to_string())),
        Query(EditStepQuery::default()),
    )
    .await
}

pub async fn get_new_mouse_click_step_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<NewMouseClickQuery>,
) -> impl IntoResponse {
    let edit_query = EditStepQuery {
        x: query.x,
        y: query.y,
        ..Default::default()
    };
    get_new_step_handler(
        State(state),
        user,
        Path((id, "mouse_click".to_string())),
        Query(edit_query),
    )
    .await
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
    form: &StepForm,
) -> Result<(), sqlx::Error> {
    match step_type {
        "branch" => {
            let condition_type = form.condition_type.as_deref().unwrap_or("pixel_rgb");
            let is_pixel = condition_type == "pixel_rgb";
            let is_window = condition_type == "window_state";

            let (x, y, r, g, b, tol) = if is_pixel {
                (
                    form.x,
                    form.y,
                    form.expected_r,
                    form.expected_g,
                    form.expected_b,
                    form.tolerance,
                )
            } else {
                (None, None, None, None, None, None)
            };

            let (ref_bm_id, sx, sy, sw, sh, m_thresh) = if condition_type == "bitmap" {
                (
                    form.reference_bitmap_id,
                    form.search_x,
                    form.search_y,
                    form.search_width,
                    form.search_height,
                    Some(form.match_threshold.unwrap_or(0.95).clamp(0.0, 1.0)),
                )
            } else {
                (None, None, None, None, None, None)
            };

            let (win_pat, win_mtype, win_ctype) = if is_window {
                (
                    form.window_match_pattern.as_deref().map(str::trim),
                    form.window_match_type.as_deref(),
                    form.window_check_type.as_deref(),
                )
            } else {
                (None, None, None)
            };

            sqlx::query(
                r#"
                INSERT INTO step_branches
                    (step_id, automation_id, condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                     reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                     window_match_pattern, window_match_type, window_check_type,
                     on_match_step_id, on_no_match_step_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
                ON CONFLICT (step_id) DO UPDATE SET
                    condition_type = EXCLUDED.condition_type,
                    x = EXCLUDED.x,
                    y = EXCLUDED.y,
                    expected_r = EXCLUDED.expected_r,
                    expected_g = EXCLUDED.expected_g,
                    expected_b = EXCLUDED.expected_b,
                    tolerance = EXCLUDED.tolerance,
                    reference_bitmap_id = EXCLUDED.reference_bitmap_id,
                    search_x = EXCLUDED.search_x,
                    search_y = EXCLUDED.search_y,
                    search_width = EXCLUDED.search_width,
                    search_height = EXCLUDED.search_height,
                    match_threshold = EXCLUDED.match_threshold,
                    window_match_pattern = EXCLUDED.window_match_pattern,
                    window_match_type = EXCLUDED.window_match_type,
                    window_check_type = EXCLUDED.window_check_type,
                    on_match_step_id = EXCLUDED.on_match_step_id,
                    on_no_match_step_id = EXCLUDED.on_no_match_step_id
                "#,
            )
            .bind(step_id)
            .bind(automation_id)
            .bind(condition_type)
            .bind(x)
            .bind(y)
            .bind(r)
            .bind(g)
            .bind(b)
            .bind(tol)
            .bind(ref_bm_id)
            .bind(sx)
            .bind(sy)
            .bind(sw)
            .bind(sh)
            .bind(m_thresh)
            .bind(win_pat)
            .bind(win_mtype)
            .bind(win_ctype)
            .bind(form.on_match_step_id.filter(|&sid| sid > 0))
            .bind(form.on_no_match_step_id.filter(|&sid| sid > 0))
            .execute(&mut **tx)
            .await?;
        }
        "mouse_scroll" => {
            let dir_str = form.direction.as_deref().unwrap_or("down");
            let direction = if dir_str.eq_ignore_ascii_case("up") {
                "up"
            } else if dir_str.eq_ignore_ascii_case("left") {
                "left"
            } else if dir_str.eq_ignore_ascii_case("right") {
                "right"
            } else {
                "down"
            };
            let scroll_amount = form.scroll_amount.unwrap_or(3).clamp(1, 100);

            let click_mode = form.click_mode.as_deref().unwrap_or("coordinates");
            let (x_mode, y_mode) = (
                form.x_mode.as_deref().unwrap_or("fixed"),
                form.y_mode.as_deref().unwrap_or("fixed"),
            );

            let (x, x_var_id, y, y_var_id, region_var_id) = if click_mode == "region" {
                (None, None, None, None, form.region_variable_id)
            } else {
                let x_val = if x_mode == "fixed" || x_mode == "literal" {
                    form.x
                } else {
                    None
                };
                let x_var = if x_mode == "variable" {
                    form.x_variable_id
                } else {
                    None
                };
                let y_val = if y_mode == "fixed" || y_mode == "literal" {
                    form.y
                } else {
                    None
                };
                let y_var = if y_mode == "variable" {
                    form.y_variable_id
                } else {
                    None
                };
                (x_val, x_var, y_val, y_var, None)
            };

            sqlx::query(
                r#"
                INSERT INTO step_mouse_scrolls (step_id, direction, scroll_amount, x, y, x_variable_id, y_variable_id, region_variable_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (step_id) DO UPDATE SET
                    direction = EXCLUDED.direction,
                    scroll_amount = EXCLUDED.scroll_amount,
                    x = EXCLUDED.x,
                    y = EXCLUDED.y,
                    x_variable_id = EXCLUDED.x_variable_id,
                    y_variable_id = EXCLUDED.y_variable_id,
                    region_variable_id = EXCLUDED.region_variable_id
                "#,
            )
            .bind(step_id)
            .bind(direction)
            .bind(scroll_amount)
            .bind(x)
            .bind(y)
            .bind(x_var_id)
            .bind(y_var_id)
            .bind(region_var_id)
            .execute(&mut **tx)
            .await?;
        }
        "window_control" => {
            let act_str = form.action.as_deref().unwrap_or("activate");
            let action = if act_str.eq_ignore_ascii_case("close") {
                "close"
            } else if act_str.eq_ignore_ascii_case("minimize") {
                "minimize"
            } else if act_str.eq_ignore_ascii_case("maximize") {
                "maximize"
            } else if act_str.eq_ignore_ascii_case("wait") {
                "wait"
            } else {
                "activate"
            };
            let match_pattern = form.match_pattern.as_deref().unwrap_or("").trim();
            let mt_str = form.match_type.as_deref().unwrap_or("contains");
            let match_type = if mt_str.eq_ignore_ascii_case("exact") {
                "exact"
            } else if mt_str.eq_ignore_ascii_case("starts_with") {
                "starts_with"
            } else {
                "contains"
            };
            let timeout_ms = ((form.timeout_seconds.unwrap_or(5.0).max(0.0)) * 1000.0) as i64;

            sqlx::query(
                r#"
                INSERT INTO step_window_controls (step_id, action, match_pattern, match_type, timeout_ms)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (step_id) DO UPDATE SET
                    action = EXCLUDED.action,
                    match_pattern = EXCLUDED.match_pattern,
                    match_type = EXCLUDED.match_type,
                    timeout_ms = EXCLUDED.timeout_ms
                "#,
            )
            .bind(step_id)
            .bind(action)
            .bind(match_pattern)
            .bind(match_type)
            .bind(timeout_ms)
            .execute(&mut **tx)
            .await?;
        }
        "type_text" => {
            let text_to_type = form.text_to_type.as_deref().unwrap_or("").trim();
            let delay_between_keys_ms = form.delay_between_keys_ms.unwrap_or(0).max(0);
            sqlx::query(
                r#"
                INSERT INTO step_type_texts (step_id, text_to_type, delay_between_keys_ms)
                VALUES ($1, $2, $3)
                ON CONFLICT (step_id) DO UPDATE SET
                    text_to_type = EXCLUDED.text_to_type,
                    delay_between_keys_ms = EXCLUDED.delay_between_keys_ms
                "#,
            )
            .bind(step_id)
            .bind(text_to_type)
            .bind(delay_between_keys_ms)
            .execute(&mut **tx)
            .await?;
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
        "ocr_extract" => {
            let set_task_out = matches!(
                form.set_as_task_output.as_deref(),
                Some("on" | "true" | "1")
            );

            sqlx::query(
                r#"
                INSERT INTO step_ocr_extracts
                    (step_id, search_x, search_y, search_width, search_height, region_variable_id, output_variable_id, set_as_task_output)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (step_id) DO UPDATE SET
                    search_x = EXCLUDED.search_x,
                    search_y = EXCLUDED.search_y,
                    search_width = EXCLUDED.search_width,
                    search_height = EXCLUDED.search_height,
                    region_variable_id = EXCLUDED.region_variable_id,
                    output_variable_id = EXCLUDED.output_variable_id,
                    set_as_task_output = EXCLUDED.set_as_task_output
                "#,
            )
            .bind(step_id)
            .bind(form.search_x)
            .bind(form.search_y)
            .bind(form.search_width)
            .bind(form.search_height)
            .bind(form.region_variable_id)
            .bind(form.output_variable_id)
            .bind(set_task_out)
            .execute(&mut **tx)
            .await?;
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn render_step_form(
    db: &PgPool,
    user: AuthUser,
    csrf_token: String,
    automation_id: i64,
    step_id: Option<i64>,
    step_type: &str,
    form: StepForm,
    error: Option<String>,
) -> axum::response::Response {
    let is_edit = step_id.is_some();
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let label = form.label.unwrap_or_default();
    let post_delay_seconds = form.post_delay_seconds.unwrap_or(0.0);

    let html_body = match step_type {
        "branch" => {
            let condition_type = form
                .condition_type
                .unwrap_or_else(|| "pixel_rgb".to_string());
            let bundle = fetch_step_options_bundle(db, automation_id, step_id).await;
            HtmlTemplate(StepBranchTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
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
                window_match_pattern: form.window_match_pattern.unwrap_or_default(),
                window_match_type: form
                    .window_match_type
                    .unwrap_or_else(|| "contains".to_string()),
                window_check_type: form
                    .window_check_type
                    .unwrap_or_else(|| "exists".to_string()),
                on_match_step_id: form.on_match_step_id,
                on_no_match_step_id: form.on_no_match_step_id,
                steps: bundle.steps,
                bitmaps: bundle.bitmaps,
                error,
                is_edit,
            })
            .into_response()
        }
        "key_press" => HtmlTemplate(StepKeyPressTemplate {
            user,
            csrf_token,
            automation_id,
            step_id,
            label,
            post_delay_seconds,
            key_combo: form.key_combo.unwrap_or_default(),
            error,
            is_edit,
        })
        .into_response(),
        "clipboard_control" => {
            let variables = fetch_automation_variables(db, automation_id).await;
            let set_task_out = form.set_as_task_output.as_deref() == Some("on")
                || form.set_as_task_output.as_deref() == Some("true")
                || form.set_as_task_output.as_deref() == Some("1");

            HtmlTemplate(StepClipboardControlTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
                label,
                post_delay_seconds,
                action: form.action.unwrap_or_else(|| "set_text".to_string()),
                text_mode: form.text_mode.unwrap_or_else(|| "literal".to_string()),
                text_to_set: form.text_to_set,
                input_variable_id: form.input_variable_id,
                output_variable_id: form.output_variable_id,
                set_as_task_output: set_task_out,
                timeout_seconds: form.timeout_seconds.unwrap_or(5.0),
                variables,
                error,
                is_edit,
            })
            .into_response()
        }
        "window_control" => HtmlTemplate(StepWindowControlTemplate {
            user,
            csrf_token,
            automation_id,
            step_id,
            label,
            post_delay_seconds,
            action: form.action.unwrap_or_else(|| "activate".to_string()),
            match_pattern: form.match_pattern.unwrap_or_default(),
            match_type: form.match_type.unwrap_or_else(|| "contains".to_string()),
            timeout_seconds: form.timeout_seconds.unwrap_or(5.0),
            error,
            is_edit,
        })
        .into_response(),
        "type_text" => HtmlTemplate(StepTypeTextTemplate {
            user,
            csrf_token,
            automation_id,
            step_id,
            label,
            post_delay_seconds,
            text_to_type: form.text_to_type.unwrap_or_default(),
            delay_between_keys_ms: form.delay_between_keys_ms.unwrap_or(0).max(0),
            error,
            is_edit,
        })
        .into_response(),
        "mouse_scroll" => {
            let variables = fetch_automation_variables(db, automation_id).await;
            HtmlTemplate(StepMouseScrollTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
                label,
                post_delay_seconds,
                direction: form.direction.unwrap_or_else(|| "down".to_string()),
                scroll_amount: form.scroll_amount.unwrap_or(3).clamp(1, 100),
                x_mode: form.x_mode.unwrap_or_else(|| "fixed".to_string()),
                x: form.x,
                x_variable_id: form.x_variable_id,
                y_mode: form.y_mode.unwrap_or_else(|| "fixed".to_string()),
                y: form.y,
                y_variable_id: form.y_variable_id,
                click_mode: form.click_mode.unwrap_or_else(|| "coordinates".to_string()),
                region_variable_id: form.region_variable_id,
                variables,
                error,
                is_edit,
            })
            .into_response()
        }
        "mouse_click" => {
            let variables = fetch_automation_variables(db, automation_id).await;
            HtmlTemplate(StepMouseClickTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
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
                error,
                is_edit,
            })
            .into_response()
        }
        "find_pixel_rgb" => {
            let variables = fetch_automation_variables(db, automation_id).await;
            HtmlTemplate(StepFindPixelRgbTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
                label,
                post_delay_seconds,
                x: form.x,
                y: form.y,
                output_variable_id: form.output_variable_id,
                variables,
                error,
                is_edit,
            })
            .into_response()
        }
        "find_bitmap" => {
            let bundle = fetch_step_options_bundle(db, automation_id, step_id).await;
            HtmlTemplate(StepFindBitmapTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
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
                bitmaps: bundle.bitmaps,
                variables: bundle.variables,
                error,
                is_edit,
            })
            .into_response()
        }
        "ocr_extract" => {
            let variables = fetch_automation_variables(db, automation_id).await;
            let set_task_out = match form.set_as_task_output.as_deref() {
                Some(v) => v == "on" || v == "true" || v == "1",
                None => true,
            };

            HtmlTemplate(StepOcrExtractTemplate {
                user,
                csrf_token,
                automation_id,
                step_id,
                label,
                post_delay_seconds,
                search_x: form.search_x,
                search_y: form.search_y,
                search_width: form.search_width,
                search_height: form.search_height,
                region_variable_id: form.region_variable_id,
                output_variable_id: form.output_variable_id,
                set_as_task_output: set_task_out,
                variables,
                error,
                is_edit,
            })
            .into_response()
        }
        _ => return Redirect::to(&format!("/automations/{}", automation_id)).into_response(),
    };

    (status, html_body).into_response()
}

#[allow(clippy::too_many_arguments)]
async fn render_step_error(
    db: &PgPool,
    user: AuthUser,
    csrf_token: String,
    automation_id: i64,
    step_id: Option<i64>,
    step_type: &str,
    form: StepForm,
    error: String,
) -> axum::response::Response {
    render_step_form(
        db,
        user,
        csrf_token,
        automation_id,
        step_id,
        step_type,
        form,
        Some(error),
    )
    .await
}

async fn validate_automation_variables_exist(
    db: &PgPool,
    automation_id: i64,
    var_ids: &[i64],
) -> Result<(), String> {
    if var_ids.is_empty() {
        return Ok(());
    }
    let mut vars = var_ids.to_vec();
    vars.sort_unstable();
    vars.dedup();

    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM automation_variables WHERE id = ANY($1) AND automation_id = $2",
    )
    .bind(&vars)
    .bind(automation_id)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    if count as usize != vars.len() {
        Err("Selected output variable is invalid for this automation.".to_string())
    } else {
        Ok(())
    }
}

async fn validate_step_form(
    db: &PgPool,
    automation_id: i64,
    step_type: &str,
    form: &StepForm,
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
        "type_text" => {
            let text = form.text_to_type.as_deref().unwrap_or("").trim();
            if text.is_empty() {
                return Err("Text to type cannot be empty.".to_string());
            }
            if let Some(delay) = form.delay_between_keys_ms
                && delay < 0
            {
                return Err("Delay between key presses cannot be negative.".to_string());
            }
        }
        "window_control" => {
            let pattern = form.match_pattern.as_deref().unwrap_or("").trim();
            if pattern.is_empty() {
                return Err("Window title match pattern cannot be empty.".to_string());
            }
            if let Some(t) = form.timeout_seconds
                && t < 0.0
            {
                return Err("Timeout cannot be negative.".to_string());
            }
        }
        "mouse_click" => {
            let is_region_mode = form.click_mode.as_deref() == Some("region")
                || (form.click_mode.is_none() && form.region_variable_id.is_some());
            if is_region_mode {
                if form.region_variable_id.is_none() {
                    return Err("Please select a valid Region variable.".to_string());
                }
            } else {
                validate_literal_or_reference(
                    form.x_mode.as_deref(),
                    form.x,
                    form.x_variable_id,
                    "X coordinate",
                )?;
                validate_literal_or_reference(
                    form.y_mode.as_deref(),
                    form.y,
                    form.y_variable_id,
                    "Y coordinate",
                )?;
            }
        }
        "find_pixel_rgb" => {
            if form.x.is_none() || form.y.is_none() {
                return Err("Please provide valid X and Y coordinates.".to_string());
            }
            if let Some(var_id) = form.output_variable_id {
                validate_automation_variables_exist(db, automation_id, &[var_id]).await?;
            }
        }
        "find_bitmap" => {
            if form.reference_bitmap_id.unwrap_or(0) <= 0 {
                return Err("Please select a reference bitmap.".to_string());
            }

            let out_vars: Vec<i64> = [
                form.output_found_variable_id,
                form.output_x_variable_id,
                form.output_y_variable_id,
                form.output_region_variable_id,
            ]
            .into_iter()
            .flatten()
            .collect();

            validate_automation_variables_exist(db, automation_id, &out_vars).await?;
        }
        "ocr_extract" => {
            let out_vars: Vec<i64> = [form.region_variable_id, form.output_variable_id]
                .into_iter()
                .flatten()
                .collect();

            validate_automation_variables_exist(db, automation_id, &out_vars).await?;
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
    form: StepForm,
) -> axum::response::Response {
    let csrf_token = user.csrf_token.clone();

    if let Err(err_msg) = validate_step_form(&state.db, automation_id, step_type, &form).await {
        return render_step_error(
            &state.db,
            user.clone(),
            csrf_token,
            automation_id,
            step_id,
            step_type,
            form,
            err_msg,
        )
        .await;
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
    Query(query): Query<EditStepQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let step_row = match sqlx::query_as::<_, BaseStepRow>(
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

    let label = step_row.label.unwrap_or_default();
    let post_delay_seconds = step_row.post_delay_ms as f64 / 1000.0;

    match step_row.step_type.as_str() {
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
        "window_control" => {
            let wc_row = sqlx::query_as::<_, WindowControlDetailRow>(
                "SELECT action, match_pattern, match_type, timeout_ms FROM step_window_controls WHERE step_id = $1",
            )
            .bind(sid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

            let action = wc_row
                .as_ref()
                .map(|r| r.action.clone())
                .unwrap_or_else(|| "activate".to_string());
            let match_pattern = wc_row
                .as_ref()
                .map(|r| r.match_pattern.clone())
                .unwrap_or_default();
            let match_type = wc_row
                .as_ref()
                .map(|r| r.match_type.clone())
                .unwrap_or_else(|| "contains".to_string());
            let timeout_seconds = wc_row
                .as_ref()
                .map(|r| r.timeout_ms as f64 / 1000.0)
                .unwrap_or(5.0);

            HtmlTemplate(StepWindowControlTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                action,
                match_pattern,
                match_type,
                timeout_seconds,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "type_text" => {
            let tt_row = sqlx::query_as::<_, TypeTextDetailRow>(
                "SELECT text_to_type, delay_between_keys_ms FROM step_type_texts WHERE step_id = $1",
            )
            .bind(sid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

            let text_to_type = tt_row
                .as_ref()
                .map(|r| r.text_to_type.clone())
                .unwrap_or_default();
            let delay_between_keys_ms = tt_row
                .as_ref()
                .map(|r| r.delay_between_keys_ms)
                .unwrap_or(0);

            HtmlTemplate(StepTypeTextTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                text_to_type,
                delay_between_keys_ms,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "find_bitmap" => {
            let (fb_row, bundle) = tokio::join!(
                sqlx::query_as::<_, FindBitmapDetailRow>(
                    "SELECT reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold, output_found_variable_id, output_x_variable_id, output_y_variable_id, output_region_variable_id FROM step_find_bitmap WHERE step_id = $1"
                )
                .bind(sid)
                .fetch_optional(&state.db),
                fetch_step_options_bundle(&state.db, id, Some(sid)),
            );

            let fb = fb_row.ok().flatten();

            HtmlTemplate(StepFindBitmapTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                reference_bitmap_id: query
                    .reference_bitmap_id
                    .or_else(|| fb.as_ref().and_then(|r| r.reference_bitmap_id)),
                search_x: query
                    .search_x
                    .or_else(|| fb.as_ref().and_then(|r| r.search_x)),
                search_y: query
                    .search_y
                    .or_else(|| fb.as_ref().and_then(|r| r.search_y)),
                search_width: query
                    .search_width
                    .or_else(|| fb.as_ref().and_then(|r| r.search_width)),
                search_height: query
                    .search_height
                    .or_else(|| fb.as_ref().and_then(|r| r.search_height)),
                match_threshold: fb.as_ref().map(|r| r.match_threshold).unwrap_or(0.95),
                output_found_variable_id: fb.as_ref().and_then(|r| r.output_found_variable_id),
                output_x_variable_id: fb.as_ref().and_then(|r| r.output_x_variable_id),
                output_y_variable_id: fb.as_ref().and_then(|r| r.output_y_variable_id),
                output_region_variable_id: fb.as_ref().and_then(|r| r.output_region_variable_id),
                bitmaps: bundle.bitmaps,
                variables: bundle.variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "branch" => {
            let (branch_row, bundle) = tokio::join!(
                sqlx::query_as::<_, BranchDetailRow>(r#"
                    SELECT condition_type, x, y, expected_r, expected_g, expected_b, tolerance,
                           reference_bitmap_id, search_x, search_y, search_width, search_height, match_threshold,
                           on_match_step_id, on_no_match_step_id,
                           window_match_pattern, window_match_type, window_check_type
                    FROM step_branches WHERE step_id = $1
                "#).bind(sid).fetch_optional(&state.db),
                fetch_step_options_bundle(&state.db, id, Some(sid)),
            );

            let br = branch_row.ok().flatten();

            HtmlTemplate(StepBranchTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                condition_type: br
                    .as_ref()
                    .map(|r| r.condition_type.clone())
                    .unwrap_or_else(|| "pixel_rgb".to_string()),
                x: query.search_x.or_else(|| br.as_ref().and_then(|r| r.x)),
                y: query.search_y.or_else(|| br.as_ref().and_then(|r| r.y)),
                expected_r: br.as_ref().and_then(|r| r.expected_r).or(Some(0)),
                expected_g: br.as_ref().and_then(|r| r.expected_g).or(Some(0)),
                expected_b: br.as_ref().and_then(|r| r.expected_b).or(Some(0)),
                tolerance: br.as_ref().and_then(|r| r.tolerance).or(Some(10)),
                reference_bitmap_id: query
                    .reference_bitmap_id
                    .or_else(|| br.as_ref().and_then(|r| r.reference_bitmap_id)),
                search_x: query
                    .search_x
                    .or_else(|| br.as_ref().and_then(|r| r.search_x)),
                search_y: query
                    .search_y
                    .or_else(|| br.as_ref().and_then(|r| r.search_y)),
                search_width: query
                    .search_width
                    .or_else(|| br.as_ref().and_then(|r| r.search_width)),
                search_height: query
                    .search_height
                    .or_else(|| br.as_ref().and_then(|r| r.search_height)),
                match_threshold: br.as_ref().and_then(|r| r.match_threshold).or(Some(0.95)),
                window_match_pattern: br
                    .as_ref()
                    .and_then(|r| r.window_match_pattern.clone())
                    .unwrap_or_default(),
                window_match_type: br
                    .as_ref()
                    .and_then(|r| r.window_match_type.clone())
                    .unwrap_or_else(|| "contains".to_string()),
                window_check_type: br
                    .as_ref()
                    .and_then(|r| r.window_check_type.clone())
                    .unwrap_or_else(|| "exists".to_string()),
                on_match_step_id: br.as_ref().and_then(|r| r.on_match_step_id),
                on_no_match_step_id: br.as_ref().and_then(|r| r.on_no_match_step_id),
                steps: bundle.steps,
                bitmaps: bundle.bitmaps,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "find_pixel_rgb" => {
            let (fp_row, variables) = tokio::join!(
                sqlx::query_as::<_, FindPixelDetailRow>(
                    "SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1"
                )
                .bind(sid)
                .fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let fp = fp_row.ok().flatten();

            HtmlTemplate(StepFindPixelRgbTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                x: fp.as_ref().and_then(|r| r.x),
                y: fp.as_ref().and_then(|r| r.y),
                output_variable_id: fp.as_ref().and_then(|r| r.output_variable_id),
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "ocr_extract" => {
            let (ocr_row, variables) = tokio::join!(
                sqlx::query_as::<_, OcrExtractDetailRow>(
                    "SELECT search_x, search_y, search_width, search_height, region_variable_id, output_variable_id, set_as_task_output FROM step_ocr_extracts WHERE step_id = $1"
                )
                .bind(sid)
                .fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let ocr = ocr_row.ok().flatten();

            HtmlTemplate(StepOcrExtractTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                search_x: query
                    .search_x
                    .or_else(|| ocr.as_ref().and_then(|r| r.search_x)),
                search_y: query
                    .search_y
                    .or_else(|| ocr.as_ref().and_then(|r| r.search_y)),
                search_width: query
                    .search_width
                    .or_else(|| ocr.as_ref().and_then(|r| r.search_width)),
                search_height: query
                    .search_height
                    .or_else(|| ocr.as_ref().and_then(|r| r.search_height)),
                region_variable_id: ocr.as_ref().and_then(|r| r.region_variable_id),
                output_variable_id: ocr.as_ref().and_then(|r| r.output_variable_id),
                set_as_task_output: ocr.as_ref().map(|r| r.set_as_task_output).unwrap_or(true),
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "mouse_scroll" => {
            let (ms_row, variables) = tokio::join!(
                sqlx::query_as::<_, MouseScrollDetailRow>("SELECT direction, scroll_amount, x, y, x_variable_id, y_variable_id, region_variable_id FROM step_mouse_scrolls WHERE step_id = $1")
                    .bind(sid).fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let ms = ms_row.ok().flatten();

            let x_var_id = ms.as_ref().and_then(|r| r.x_variable_id);
            let y_var_id = ms.as_ref().and_then(|r| r.y_variable_id);
            let region_var_id = ms.as_ref().and_then(|r| r.region_variable_id);

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

            HtmlTemplate(StepMouseScrollTemplate {
                user,
                csrf_token,
                automation_id: id,
                step_id: Some(sid),
                label,
                post_delay_seconds,
                direction: ms
                    .as_ref()
                    .map(|r| r.direction.clone())
                    .unwrap_or_else(|| "down".to_string()),
                scroll_amount: ms.as_ref().map(|r| r.scroll_amount).unwrap_or(3),
                x_mode,
                x: query.search_x.or_else(|| ms.as_ref().and_then(|r| r.x)),
                x_variable_id: x_var_id,
                y_mode,
                y: query.search_y.or_else(|| ms.as_ref().and_then(|r| r.y)),
                y_variable_id: y_var_id,
                click_mode,
                region_variable_id: region_var_id,
                variables,
                error: None,
                is_edit: true,
            })
            .into_response()
        }
        "mouse_click" => {
            let (mc_row, variables) = tokio::join!(
                sqlx::query_as::<_, MouseClickDetailRow>("SELECT x, y, x_variable_id, y_variable_id, region_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
                    .bind(sid).fetch_optional(&state.db),
                fetch_automation_variables(&state.db, id),
            );

            let mc = mc_row.ok().flatten();

            let x_var_id = mc.as_ref().and_then(|r| r.x_variable_id);
            let y_var_id = mc.as_ref().and_then(|r| r.y_variable_id);
            let region_var_id = mc.as_ref().and_then(|r| r.region_variable_id);

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
                x: mc.as_ref().and_then(|r| r.x),
                x_variable_id: x_var_id,
                y_mode,
                y: mc.as_ref().and_then(|r| r.y),
                y_variable_id: y_var_id,
                click_mode,
                region_variable_id: region_var_id,
                button: mc
                    .as_ref()
                    .map(|r| r.button.clone())
                    .unwrap_or_else(|| "left".to_string()),
                click_type: mc
                    .as_ref()
                    .map(|r| r.click_type.clone())
                    .unwrap_or_else(|| "single".to_string()),
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
    CsrfForm(form): CsrfForm<StepForm>,
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
    CsrfForm(form): CsrfForm<StepForm>,
) -> impl IntoResponse {
    if !user.role.can_edit() {
        return Redirect::to(&format!("/automations/{}", id)).into_response();
    }

    let step_type_row = sqlx::query_as::<_, StepTypeRow>(
        "SELECT step_type FROM automation_steps WHERE id = $1 AND automation_id = $2 AND deleted_at IS NULL",
    )
    .bind(sid)
    .bind(id)
    .fetch_optional(&state.db)
    .await;

    let step_type = match step_type_row {
        Ok(Some(r)) => r.step_type,
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

    let steps = sqlx::query_as::<_, StepPosRow>(
        "SELECT id, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&mut *tx)
    .await?;

    let current_idx = steps.iter().position(|r| r.id == step_id);
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

    let pos_curr = steps[idx].position;
    let pos_target = steps[target_idx].position;
    let target_id = steps[target_idx].id;

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
    let steps = sqlx::query_as::<_, StepPosRow>(
        "SELECT id, position FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await?;

    let mut needs_compaction = false;
    for i in 0..steps.len().saturating_sub(1) {
        if (steps[i + 1].position - steps[i].position).abs() < 0.0001 {
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

    let steps = sqlx::query_as::<_, StepIdRow>(
        "SELECT id FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&mut *tx)
    .await?;

    for (i, row) in steps.iter().enumerate() {
        let new_pos = (i as f64 + 1.0) * 10.0;
        sqlx::query("UPDATE automation_steps SET position = $1 WHERE id = $2")
            .bind(new_pos)
            .bind(row.id)
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
    fn test_generate_clipboard_control_summary() {
        let s1 = generate_clipboard_control_summary(
            "set_text",
            Some("Hello World"),
            None,
            None,
            false,
            5000,
        );
        assert_eq!(s1, "Set clipboard to \"Hello World\"");

        let s2 =
            generate_clipboard_control_summary("set_text", None, Some("my_var"), None, false, 5000);
        assert_eq!(s2, "Set clipboard to «my_var»");

        let s3 =
            generate_clipboard_control_summary("get_text", None, None, Some("out_var"), true, 5000);
        assert_eq!(
            s3,
            "Read clipboard text → store in «out_var» (set as task output)"
        );

        let s4 = generate_clipboard_control_summary("clear", None, None, None, false, 5000);
        assert_eq!(s4, "Clear clipboard");

        let s5 = generate_clipboard_control_summary(
            "wait_for_change",
            None,
            None,
            Some("out_var"),
            false,
            5000,
        );
        assert_eq!(
            s5,
            "Wait for clipboard change [timeout 5.0s] → store in «out_var»"
        );
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
            None,
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
            None,
            None,
            Some("Step 5 (Click)"),
            Some("Next Step"),
        );
        assert_eq!(
            s2,
            "BRANCH: if bitmap «submit_btn» found → go to Step 5 (Click), else → go to Next Step"
        );

        let s3 = generate_branch_summary(
            "window_state",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("Calculator"),
            Some("active"),
            Some("Step 3 (Focus)"),
            Some("Next Step"),
        );
        assert_eq!(
            s3,
            "BRANCH: if window \"Calculator\" active → go to Step 3 (Focus), else → go to Next Step"
        );

        let s4 = generate_branch_summary(
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
            None,
            None,
        );
        assert_eq!(
            s4,
            "BRANCH: if pixel at (0, 0) ≈ RGB(0,0,0) ±0 → go to Next Step, else → go to Next Step"
        );

        let s5 = generate_branch_summary(
            "bitmap", None, None, None, None, None, None, None, None, None, None, None,
        );
        assert_eq!(
            s5,
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
            object_storage_key: "bitmaps/test.png".to_string(),
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

    #[test]
    fn test_step_helpers_sanitization_summaries_and_branch_validation() {
        // 1. Label sanitization
        assert_eq!(sanitize_label(None), None);
        assert_eq!(sanitize_label(Some("")), None);
        assert_eq!(sanitize_label(Some("   ")), None);
        assert_eq!(
            sanitize_label(Some("  Click Submit  ")),
            Some("Click Submit")
        );

        // 2. Seconds to post delay ms conversion
        assert_eq!(seconds_to_post_delay_ms(None), 0);
        assert_eq!(seconds_to_post_delay_ms(Some(0.0)), 0);
        assert_eq!(seconds_to_post_delay_ms(Some(-1.5)), 0);
        assert_eq!(seconds_to_post_delay_ms(Some(0.25)), 250);
        assert_eq!(seconds_to_post_delay_ms(Some(3.0)), 3000);

        // 3. Type text summary formatting and truncation
        assert_eq!(
            generate_type_text_summary("Hello World", 0),
            "Type \"Hello World\""
        );
        assert_eq!(
            generate_type_text_summary(
                "Type text with long content exceeding thirty chars limit",
                0
            ),
            "Type \"Type text with long content ex...\""
        );
        assert_eq!(
            generate_type_text_summary("Hello", 50),
            "Type \"Hello\" (50ms key delay)"
        );
        assert_eq!(
            generate_type_text_summary("  Padded text  ", 10),
            "Type \"Padded text\" (10ms key delay)"
        );

        // 4. Mouse button & click type sanitization
        assert_eq!(sanitize_mouse_button(Some("right")), "right");
        assert_eq!(sanitize_mouse_button(Some("middle")), "middle");
        assert_eq!(sanitize_mouse_button(Some("left")), "left");
        assert_eq!(sanitize_mouse_button(Some("invalid")), "left");
        assert_eq!(sanitize_mouse_button(None), "left");

        assert_eq!(sanitize_click_type(Some("double")), "double");
        assert_eq!(sanitize_click_type(Some("single")), "single");
        assert_eq!(sanitize_click_type(Some("invalid")), "single");
        assert_eq!(sanitize_click_type(None), "single");

        // 5. Branch form validation
        let mut form = MouseClickStepForm {
            step_type: Some("branch".to_string()),
            label: None,
            post_delay_seconds: None,
            x_mode: None,
            x: Some(100),
            x_variable_id: None,
            y_mode: None,
            y: Some(200),
            y_variable_id: None,
            click_mode: None,
            region_variable_id: None,
            button: None,
            click_type: None,
            key_combo: None,
            text_to_type: None,
            delay_between_keys_ms: None,
            condition_type: Some("pixel_rgb".to_string()),
            expected_r: Some(255),
            expected_g: Some(255),
            expected_b: Some(255),
            tolerance: Some(5),
            reference_bitmap_id: None,
            search_x: None,
            search_y: None,
            search_width: None,
            search_height: None,
            match_threshold: None,
            on_match_step_id: None,
            on_no_match_step_id: Some(2),
            set_as_task_output: None,
            output_variable_id: None,
            output_found_variable_id: None,
            output_x_variable_id: None,
            output_y_variable_id: None,
            output_region_variable_id: None,
            action: None,
            match_pattern: None,
            match_type: None,
            text_mode: None,
            text_to_set: None,
            input_variable_id: None,
            timeout_seconds: None,
            direction: None,
            scroll_amount: None,
            window_match_pattern: None,
            window_match_type: None,
            window_check_type: None,
        };

        // Missing on_match_step_id
        assert_eq!(
            validate_branch_form(&form, "pixel_rgb"),
            Some(
                "Both match and no-match target steps are required for branch conditions."
                    .to_string()
            )
        );

        // Target step ID <= 0
        form.on_match_step_id = Some(0);
        assert_eq!(
            validate_branch_form(&form, "pixel_rgb"),
            Some(
                "Both match and no-match target steps are required for branch conditions."
                    .to_string()
            )
        );

        // Valid targets, missing X for pixel_rgb
        form.on_match_step_id = Some(1);
        form.x = None;
        assert_eq!(
            validate_branch_form(&form, "pixel_rgb"),
            Some("Please provide valid X and Y coordinates for pixel RGB condition.".to_string())
        );

        // Valid pixel_rgb branch form
        form.x = Some(100);
        assert_eq!(validate_branch_form(&form, "pixel_rgb"), None);

        // Missing reference_bitmap_id for bitmap condition
        form.condition_type = Some("bitmap".to_string());
        form.reference_bitmap_id = None;
        assert_eq!(
            validate_branch_form(&form, "bitmap"),
            Some("Please select a reference bitmap for bitmap condition.".to_string())
        );

        // Valid bitmap branch form
        form.reference_bitmap_id = Some(10);
        assert_eq!(validate_branch_form(&form, "bitmap"), None);
    }

    #[test]
    fn test_generate_ocr_extract_summary() {
        let s1 = generate_ocr_extract_summary(
            Some(100),
            Some(200),
            Some(300),
            Some(150),
            None,
            Some("extracted_text"),
            true,
        );
        assert_eq!(
            s1,
            "OCR Extract from region (100, 200, 300x150) → store in «extracted_text» [task output]"
        );

        let s2 = generate_ocr_extract_summary(
            None,
            None,
            None,
            None,
            Some("header_region"),
            None,
            false,
        );
        assert_eq!(s2, "OCR Extract from region «header_region»");

        let s3 = generate_ocr_extract_summary(None, None, None, None, None, None, false);
        assert_eq!(s3, "OCR Extract from full screen");
    }
}
