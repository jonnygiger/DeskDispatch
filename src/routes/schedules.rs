use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;
use std::borrow::Cow;

use super::auth::HtmlTemplate;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, FlashMessage, RequireEditor, log_audit};
use crate::de::deserialize_option_number;

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

#[derive(Debug, Clone, FromRow)]
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
    pub wol_enabled: bool,
    pub wol_warmup_secs: i32,
    pub last_wol_sent_at: Option<DateTime<Utc>>,
    pub is_enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_by: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct EditScheduleRow {
    pub automation_id: i64,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub worker_group_id: Option<i64>,
    pub overlap_policy: String,
    pub max_queue_age_secs: Option<i32>,
    pub wol_enabled: bool,
    pub wol_warmup_secs: i32,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct ToggleScheduleRow {
    pub is_enabled: bool,
    pub cron_expression: String,
    pub timezone: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct WolScheduleRow {
    pub id: i64,
    pub worker_group_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DueScheduleRow {
    pub id: i64,
    pub automation_id: i64,
    pub cron_expression: String,
    pub timezone: String,
    pub worker_group_id: Option<i64>,
    pub overlap_policy: String,
    pub max_queue_age_secs: Option<i32>,
    pub next_run_at: DateTime<Utc>,
}

impl ScheduleItem {
    pub fn formatted_next_run(&self) -> Cow<'static, str> {
        match self.next_run_at {
            Some(dt) => Cow::Owned(dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            None if self.is_enabled => Cow::Borrowed("Pending"),
            None => Cow::Borrowed("Disabled"),
        }
    }

    pub fn formatted_last_run(&self) -> Cow<'static, str> {
        match self.last_run_at {
            Some(dt) => Cow::Owned(dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            None => Cow::Borrowed("Never"),
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
    pub wol_enabled: bool,
    pub wol_warmup_secs: i32,
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

    pub fn build_error(
        user: AuthUser,
        schedule_id: Option<i64>,
        form: &ScheduleForm,
        automations: Vec<ScheduleAutomationOption>,
        worker_groups: Vec<ScheduleWorkerGroupOption>,
        error: impl Into<String>,
    ) -> Self {
        let (name, cron, tz, wol_warmup) = form.normalized();
        Self {
            user,
            is_edit: schedule_id.is_some(),
            schedule_id,
            automation_id: Some(form.automation_id),
            name: name.to_string(),
            cron_expression: cron.to_string(),
            timezone: tz.to_string(),
            worker_group_id: form.worker_group_id,
            overlap_policy: form.overlap_policy.clone(),
            max_queue_age_secs: form.max_queue_age_secs,
            wol_enabled: form.wol_enabled,
            wol_warmup_secs: wol_warmup,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some(error.into()),
        }
    }
}

#[derive(Deserialize)]
pub struct ScheduleForm {
    pub automation_id: i64,
    pub name: String,
    pub cron_expression: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub worker_group_id: Option<i64>,
    #[serde(default = "default_overlap_policy")]
    pub overlap_policy: String,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub max_queue_age_secs: Option<i32>,
    #[serde(default)]
    pub wol_enabled: bool,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub wol_warmup_secs: Option<i32>,
    #[serde(default)]
    pub is_enabled: bool,
}

impl ScheduleForm {
    pub fn normalized(&self) -> (&str, &str, &str, i32) {
        let name = self.name.trim();
        let cron = self.cron_expression.trim();
        let tz = self.timezone.trim();
        let tz = if tz.is_empty() { "UTC" } else { tz };
        let wol_warmup = self.wol_warmup_secs.unwrap_or(0).max(0);
        (name, cron, tz, wol_warmup)
    }
}

fn default_overlap_policy() -> String {
    "allow".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

pub struct ValidatedScheduleInput<'a> {
    pub name: &'a str,
    pub cron_expression: &'a str,
    pub timezone: &'a str,
    pub wol_warmup_secs: i32,
    pub next_run_at: Option<DateTime<Utc>>,
}

pub struct ScheduleValidationError(pub Box<HtmlTemplate<ScheduleFormTemplate>>);

impl IntoResponse for ScheduleValidationError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::UNPROCESSABLE_ENTITY, *self.0).into_response()
    }
}

async fn validate_schedule_form<'a>(
    db: &sqlx::PgPool,
    user: AuthUser,
    form: &'a ScheduleForm,
    schedule_id: Option<i64>,
) -> Result<ValidatedScheduleInput<'a>, ScheduleValidationError> {
    let (name, cron_expression, timezone, wol_warmup_secs) = form.normalized();

    if name.is_empty() || cron_expression.is_empty() {
        let (automations, worker_groups) = fetch_schedule_options(db).await;
        let err_template = ScheduleFormTemplate::build_error(
            user,
            schedule_id,
            form,
            automations,
            worker_groups,
            "Schedule Name and Cron Expression are required.",
        );
        return Err(ScheduleValidationError(Box::new(HtmlTemplate(
            err_template,
        ))));
    }

    let now = Utc::now();
    let parsed_dt = match compute_next_run_at(cron_expression, timezone, &now) {
        Ok(dt) => dt,
        Err(msg) => {
            let (automations, worker_groups) = fetch_schedule_options(db).await;
            let err_template = ScheduleFormTemplate::build_error(
                user,
                schedule_id,
                form,
                automations,
                worker_groups,
                msg,
            );
            return Err(ScheduleValidationError(Box::new(HtmlTemplate(
                err_template,
            ))));
        }
    };

    let next_run_at = if form.is_enabled {
        Some(parsed_dt)
    } else {
        None
    };

    Ok(ValidatedScheduleInput {
        name,
        cron_expression,
        timezone,
        wol_warmup_secs,
        next_run_at,
    })
}

use chrono::{Datelike, Timelike};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CronSchedule {
    sec_mask: u64,
    min_mask: u64,
    hour_mask: u32,
    dom_mask: u32,
    month_mask: u16,
    dow_mask: u8,
    is_6_fields: bool,
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

fn parse_field_val(s: &str, is_dow: bool, is_month: bool) -> Result<u32, ()> {
    let s_trimmed = s.trim();
    if s_trimmed.is_empty() {
        return Err(());
    }

    if let Ok(val) = s_trimmed.parse::<u32>() {
        return Ok(val);
    }

    if is_month {
        if let Some(idx) = MONTHS
            .iter()
            .position(|&m| m.eq_ignore_ascii_case(s_trimmed))
        {
            return Ok((idx + 1) as u32);
        }
    } else if is_dow {
        if let Some(idx) = DAYS.iter().position(|&d| d.eq_ignore_ascii_case(s_trimmed)) {
            return Ok(idx as u32);
        }
    }

    Err(())
}

fn parse_cron_field(
    field_str: &str,
    min_val: u32,
    max_val: u32,
    is_dow: bool,
    is_month: bool,
) -> Result<u64, &'static str> {
    let mut mask = 0u64;

    for part in field_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("Invalid empty component in cron field");
        }

        let (range_str, step) = if let Some((r, s)) = part.split_once('/') {
            let step_val = s
                .trim()
                .parse::<u32>()
                .map_err(|_| "Invalid step in cron field")?;
            if step_val == 0 {
                return Err("Step value cannot be zero in cron field");
            }
            (r.trim(), step_val)
        } else {
            (part, 1u32)
        };

        let (start, end) = if range_str == "*" {
            (min_val, max_val)
        } else if let Some((st, en)) = range_str.split_once('-') {
            let start = parse_field_val(st, is_dow, is_month)
                .map_err(|_| "Invalid start range in cron field")?;
            let end = parse_field_val(en, is_dow, is_month)
                .map_err(|_| "Invalid end range in cron field")?;
            (start, end)
        } else {
            let val = parse_field_val(range_str, is_dow, is_month)
                .map_err(|_| "Invalid value in cron field")?;
            if part.contains('/') {
                (val, max_val)
            } else {
                (val, val)
            }
        };

        if start > end || start < min_val || end > max_val {
            return Err("Value out of bounds in cron field");
        }

        let mut curr = start;
        while curr <= end {
            let bit_idx = if is_dow && curr == 7 { 0 } else { curr };
            mask |= 1u64 << bit_idx;
            curr += step;
        }
    }

    if mask == 0 {
        return Err("Cron field produced an empty mask");
    }

    Ok(mask)
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self, &'static str> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Err("Empty cron expression");
        }

        let mut fields = [""; 6];
        let mut count = 0;
        for part in trimmed.split_whitespace() {
            if count < 6 {
                fields[count] = part;
            }
            count += 1;
        }

        match count {
            5 => {
                let min_mask = parse_cron_field(fields[0], 0, 59, false, false)?;
                let hour_mask = parse_cron_field(fields[1], 0, 23, false, false)? as u32;
                let dom_mask = parse_cron_field(fields[2], 1, 31, false, false)? as u32;
                let month_mask = parse_cron_field(fields[3], 1, 12, false, true)? as u16;
                let dow_mask = parse_cron_field(fields[4], 0, 7, true, false)? as u8;

                Ok(Self {
                    sec_mask: 1u64, // 0th second
                    min_mask,
                    hour_mask,
                    dom_mask,
                    month_mask,
                    dow_mask,
                    is_6_fields: false,
                })
            }
            6 => {
                let sec_mask = parse_cron_field(fields[0], 0, 59, false, false)?;
                let min_mask = parse_cron_field(fields[1], 0, 59, false, false)?;
                let hour_mask = parse_cron_field(fields[2], 0, 23, false, false)? as u32;
                let dom_mask = parse_cron_field(fields[3], 1, 31, false, false)? as u32;
                let month_mask = parse_cron_field(fields[4], 1, 12, false, true)? as u16;
                let dow_mask = parse_cron_field(fields[5], 0, 7, true, false)? as u8;

                Ok(Self {
                    sec_mask,
                    min_mask,
                    hour_mask,
                    dom_mask,
                    month_mask,
                    dow_mask,
                    is_6_fields: true,
                })
            }
            _ => Err("Invalid field count: expected 5 or 6 fields"),
        }
    }
}

#[inline]
fn reset_time_to_start_of_day<Tz: chrono::TimeZone>(
    dt: chrono::DateTime<Tz>,
) -> chrono::DateTime<Tz> {
    dt.with_hour(0)
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(dt)
}

#[inline]
fn reset_time_to_start_of_hour<Tz: chrono::TimeZone>(
    dt: chrono::DateTime<Tz>,
) -> chrono::DateTime<Tz> {
    dt.with_minute(0)
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(dt)
}

#[inline]
fn reset_time_to_start_of_minute<Tz: chrono::TimeZone>(
    dt: chrono::DateTime<Tz>,
) -> chrono::DateTime<Tz> {
    dt.with_second(0)
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(dt)
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
        .map_err(|_| format!("Invalid timezone: '{tz_name}'"))?;

    let schedule = CronSchedule::parse(cron_expr).map_err(|e| e.to_string())?;

    let local_from = from_dt.with_timezone(&tz);
    let mut curr = if schedule.is_6_fields {
        local_from + chrono::Duration::seconds(1)
    } else {
        reset_time_to_start_of_minute(local_from) + chrono::Duration::minutes(1)
    };

    let max_steps = if schedule.is_6_fields {
        315_360_000
    } else {
        5_256_000
    };
    let mut steps = 0;

    let is_dom_restricted = schedule.dom_mask != 0xFFFF_FFFE;
    let is_dow_restricted = schedule.dow_mask != 0x7F;

    while steps < max_steps {
        steps += 1;

        let month = curr.month();
        if (schedule.month_mask & (1u16 << month)) == 0 {
            let year = if month == 12 {
                curr.year() + 1
            } else {
                curr.year()
            };
            let next_m = if month == 12 { 1 } else { month + 1 };
            if let Some(next_dt) = curr
                .with_day(1)
                .and_then(|d| d.with_month(next_m))
                .and_then(|d| d.with_year(year))
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
            {
                curr = next_dt;
                continue;
            }
        }

        let dom = curr.day();
        let dow = curr.weekday().num_days_from_sunday();

        let day_matches = if is_dom_restricted && is_dow_restricted {
            (schedule.dom_mask & (1u32 << dom)) != 0 || (schedule.dow_mask & (1u8 << dow)) != 0
        } else {
            (schedule.dom_mask & (1u32 << dom)) != 0 && (schedule.dow_mask & (1u8 << dow)) != 0
        };

        if !day_matches {
            curr = reset_time_to_start_of_day(curr + chrono::Duration::days(1));
            continue;
        }

        let hour = curr.hour();
        if (schedule.hour_mask & (1u32 << hour)) == 0 {
            curr = reset_time_to_start_of_hour(curr + chrono::Duration::hours(1));
            continue;
        }

        let min = curr.minute();
        if (schedule.min_mask & (1u64 << min)) == 0 {
            curr = reset_time_to_start_of_minute(curr + chrono::Duration::minutes(1));
            continue;
        }

        let sec = curr.second();
        if (schedule.sec_mask & (1u64 << sec)) == 0 {
            curr += chrono::Duration::seconds(1);
            continue;
        }

        return Ok(curr.with_timezone(&Utc));
    }

    Err("No matching cron occurrence found".to_string())
}

#[tracing::instrument(skip(state, user))]
pub async fn get_schedules_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    let schedules = match sqlx::query_as::<_, ScheduleItem>(
        r#"
        SELECT
            s.id, s.automation_id, a.name AS automation_name,
            s.name, s.cron_expression, s.timezone,
            s.worker_group_id, wg.name AS worker_group_name,
            s.overlap_policy, s.max_queue_age_secs,
            s.wol_enabled, s.wol_warmup_secs, s.last_wol_sent_at,
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
        Ok(schedules) => schedules,
        Err(e) => {
            tracing::error!("Failed to fetch schedules: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load schedules",
            )
                .into_response();
        }
    };

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
        wol_enabled: false,
        wol_warmup_secs: 300,
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
    let validated = match validate_schedule_form(&state.db, user.clone(), &form, None).await {
        Ok(v) => v,
        Err(err) => return err.into_response(),
    };

    let row_res = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO schedules (
            automation_id, name, cron_expression, timezone,
            worker_group_id, overlap_policy, max_queue_age_secs,
            wol_enabled, wol_warmup_secs,
            is_enabled, next_run_at, created_by
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING id
        "#,
    )
    .bind(form.automation_id)
    .bind(validated.name)
    .bind(validated.cron_expression)
    .bind(validated.timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.wol_enabled)
    .bind(validated.wol_warmup_secs)
    .bind(form.is_enabled)
    .bind(validated.next_run_at)
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let schedule_id: i64 = match row_res {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to create schedule: {e}");
            let (automations, worker_groups) = fetch_schedule_options(&state.db).await;
            return HtmlTemplate(ScheduleFormTemplate::build_error(
                user,
                None,
                &form,
                automations,
                worker_groups,
                "Database error creating schedule.",
            ))
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
            "name": validated.name,
            "cron_expression": validated.cron_expression,
            "timezone": validated.timezone,
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
    let (schedule_res, automations_res, worker_groups_res) = tokio::join!(
        sqlx::query_as::<_, EditScheduleRow>(
            r#"
            SELECT automation_id, name, cron_expression, timezone, worker_group_id,
                   overlap_policy, max_queue_age_secs, wol_enabled, wol_warmup_secs, is_enabled
            FROM schedules
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&state.db),
        sqlx::query_as::<_, ScheduleAutomationOption>(
            "SELECT id, name FROM automations WHERE status != 'archived' ORDER BY name ASC",
        )
        .fetch_all(&state.db),
        sqlx::query_as::<_, ScheduleWorkerGroupOption>(
            "SELECT id, name FROM worker_groups ORDER BY name ASC",
        )
        .fetch_all(&state.db),
    );

    let row = match schedule_res {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "Schedule not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch schedule: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let automations = automations_res.unwrap_or_default();
    let worker_groups = worker_groups_res.unwrap_or_default();

    HtmlTemplate(ScheduleFormTemplate {
        user,
        is_edit: true,
        schedule_id: Some(id),
        automation_id: Some(row.automation_id),
        name: row.name,
        cron_expression: row.cron_expression,
        timezone: row.timezone,
        worker_group_id: row.worker_group_id,
        overlap_policy: row.overlap_policy,
        max_queue_age_secs: row.max_queue_age_secs,
        wol_enabled: row.wol_enabled,
        wol_warmup_secs: row.wol_warmup_secs,
        is_enabled: row.is_enabled,
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
    let validated = match validate_schedule_form(&state.db, user.clone(), &form, Some(id)).await {
        Ok(v) => v,
        Err(err) => return err.into_response(),
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
            wol_enabled = $8,
            wol_warmup_secs = $9,
            is_enabled = $10,
            next_run_at = $11
        WHERE id = $12
        "#,
    )
    .bind(form.automation_id)
    .bind(validated.name)
    .bind(validated.cron_expression)
    .bind(validated.timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.wol_enabled)
    .bind(validated.wol_warmup_secs)
    .bind(form.is_enabled)
    .bind(validated.next_run_at)
    .bind(id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update schedule: {e}");
        let (automations, worker_groups) = fetch_schedule_options(&state.db).await;
        return HtmlTemplate(ScheduleFormTemplate::build_error(
            user,
            Some(id),
            &form,
            automations,
            worker_groups,
            "Database error updating schedule.",
        ))
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
            "name": validated.name,
            "cron_expression": validated.cron_expression,
            "timezone": validated.timezone,
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
    let row = match sqlx::query_as::<_, ToggleScheduleRow>(
        "SELECT is_enabled, cron_expression, timezone FROM schedules WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "Schedule not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch schedule for toggle: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let current_enabled = row.is_enabled;
    let cron_expr = row.cron_expression;
    let timezone = row.timezone;
    let new_enabled = !current_enabled;

    let now = Utc::now();
    let next_run_at = if new_enabled {
        match compute_next_run_at(&cron_expr, &timezone, &now) {
            Ok(next_dt) => Some(next_dt),
            Err(err_msg) => {
                tracing::warn!(schedule_id = id, error = %err_msg, "Refusing to enable schedule with invalid cron expression");
                let flash = FlashMessage::error(format!("Cannot enable schedule: {err_msg}"));
                let (c1, c2) = crate::auth::build_flash_cookie(&flash);
                return (
                    [
                        (axum::http::header::SET_COOKIE, c1),
                        (axum::http::header::SET_COOKIE, c2),
                    ],
                    Redirect::to("/schedules"),
                )
                    .into_response();
            }
        }
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
        tracing::error!("Failed to toggle schedule: {e}");
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
            tracing::error!("Failed to delete schedule: {e}");
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

    // Phase 1: Send Wake-on-LAN magic packets for schedules entering their warm-up window
    let wol_due_schedules = sqlx::query_as::<_, WolScheduleRow>(
        r#"
        SELECT id, worker_group_id
        FROM schedules
        WHERE is_enabled = true
          AND wol_enabled = true
          AND next_run_at IS NOT NULL
          AND now() >= (next_run_at - (wol_warmup_secs * INTERVAL '1 second'))
          AND (last_wol_sent_at IS NULL OR last_wol_sent_at < (next_run_at - (wol_warmup_secs * INTERVAL '1 second')))
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    if !wol_due_schedules.is_empty() {
        let wol_schedule_ids: Vec<i64> = wol_due_schedules.iter().map(|s| s.id).collect();

        #[derive(Debug, sqlx::FromRow)]
        struct WolTargetRow {
            schedule_id: i64,
            mac_address: String,
        }

        // Single combined query fetching target MAC addresses for all due WOL schedules
        let wol_targets = sqlx::query_as::<_, WolTargetRow>(
            r#"
            SELECT DISTINCT s.id AS schedule_id, w.mac_address
            FROM schedules s
            LEFT JOIN worker_group_members m ON s.worker_group_id = m.group_id
            JOIN task_worker_pcs w ON (
                (s.worker_group_id IS NOT NULL AND m.worker_id = w.id)
                OR (s.worker_group_id IS NULL)
            )
            WHERE s.id = ANY($1)
              AND w.mac_address IS NOT NULL
              AND w.mac_address != ''
            "#,
        )
        .bind(&wol_schedule_ids)
        .fetch_all(&mut *tx)
        .await?;

        for target in wol_targets {
            if let Err(e) = crate::wol::send_wol_packet(&target.mac_address).await {
                tracing::warn!(
                    schedule_id = target.schedule_id,
                    mac = %target.mac_address,
                    error = %e,
                    "Failed to send schedule WOL warm-up magic packet"
                );
            } else {
                tracing::info!(
                    schedule_id = target.schedule_id,
                    mac = %target.mac_address,
                    "Dispatched schedule WOL warm-up magic packet"
                );
            }
        }

        // Batch update all processed WOL schedules in one statement
        sqlx::query("UPDATE schedules SET last_wol_sent_at = now() WHERE id = ANY($1)")
            .bind(&wol_schedule_ids)
            .execute(&mut *tx)
            .await?;
    }

    // Phase 2: Process due task runs
    let due_schedules = sqlx::query_as::<_, DueScheduleRow>(
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

    if due_schedules.is_empty() {
        tx.commit().await?;
        return Ok(0);
    }

    // Batch query active task runs for all due schedules with overlap_policy='skip'
    let skip_sched_ids: Vec<i64> = due_schedules
        .iter()
        .filter(|s| s.overlap_policy == "skip")
        .map(|s| s.id)
        .collect();

    let active_sched_set: std::collections::HashSet<i64> = if !skip_sched_ids.is_empty() {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT DISTINCT schedule_id
            FROM task_runs
            WHERE schedule_id = ANY($1)
              AND status IN ('queued', 'running', 'cancelling')
            "#,
        )
        .bind(&skip_sched_ids)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect()
    } else {
        std::collections::HashSet::new()
    };

    let now = Utc::now();
    let num_due = due_schedules.len();

    let mut new_auto_ids = Vec::with_capacity(num_due);
    let mut new_sched_ids = Vec::with_capacity(num_due);
    let mut new_target_wg_ids = Vec::with_capacity(num_due);

    let mut update_sched_ids = Vec::with_capacity(num_due);
    let mut update_next_runs = Vec::with_capacity(num_due);
    let mut update_was_queued = Vec::with_capacity(num_due);

    for sched in due_schedules {
        let mut should_queue = true;

        // 1. Check max_queue_age_secs expiry
        if let Some(max_age) = sched.max_queue_age_secs {
            let age_secs = (now - sched.next_run_at).num_seconds();
            if age_secs > max_age as i64 {
                tracing::warn!(
                    schedule_id = sched.id,
                    age_secs = age_secs,
                    max_queue_age_secs = max_age,
                    "Scheduled run expired because it exceeded max_queue_age_secs; skipping execution"
                );
                should_queue = false;
            }
        }

        // 2. Check overlap policy if 'skip'
        if should_queue && sched.overlap_policy == "skip" && active_sched_set.contains(&sched.id) {
            tracing::warn!(
                schedule_id = sched.id,
                "Active task run exists for schedule with overlap_policy='skip'; skipping execution"
            );
            should_queue = false;
        }

        if should_queue {
            new_auto_ids.push(sched.automation_id);
            new_sched_ids.push(sched.id);
            new_target_wg_ids.push(sched.worker_group_id);
        }

        // Recompute next run time loudly on failure
        let next_run = match compute_next_run_at(&sched.cron_expression, &sched.timezone, &now) {
            Ok(next_dt) => Some(next_dt),
            Err(e) => {
                tracing::error!(
                    schedule_id = sched.id,
                    cron_expression = %sched.cron_expression,
                    timezone = %sched.timezone,
                    error = %e,
                    "FAILED to compute next_run_at for schedule! Schedule next_run_at will be set to NULL."
                );
                None
            }
        };

        update_sched_ids.push(sched.id);
        update_next_runs.push(next_run);
        update_was_queued.push(should_queue);
    }

    let queued_count = new_sched_ids.len() as u64;

    // Batch insert all queued task runs in a single statement
    if !new_sched_ids.is_empty() {
        sqlx::query(
            r#"
            INSERT INTO task_runs (automation_id, schedule_id, target_worker_group_id, status, queued_at)
            SELECT u.auto_id, u.sched_id, u.target_wg_id, 'queued', now()
            FROM UNNEST($1::BIGINT[], $2::BIGINT[], $3::BIGINT[]) AS u(auto_id, sched_id, target_wg_id)
            "#,
        )
        .bind(&new_auto_ids)
        .bind(&new_sched_ids)
        .bind(&new_target_wg_ids)
        .execute(&mut *tx)
        .await?;
    }

    // Single unified batch update for all processed schedules
    if !update_sched_ids.is_empty() {
        sqlx::query(
            r#"
            UPDATE schedules AS s
            SET last_run_at = CASE WHEN u.was_queued THEN now() ELSE s.last_run_at END,
                next_run_at = u.next_run
            FROM UNNEST($1::BIGINT[], $2::TIMESTAMPTZ[], $3::BOOLEAN[]) AS u(id, next_run, was_queued)
            WHERE s.id = u.id
            "#,
        )
        .bind(&update_sched_ids)
        .bind(&update_next_runs)
        .bind(&update_was_queued)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    if queued_count > 0 {
        tracing::info!(count = queued_count, "Queued scheduled task runs");
    }

    Ok(queued_count)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
            wol_enabled: false,
            wol_warmup_secs: 300,
            last_wol_sent_at: None,
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
            wol_enabled: true,
            wol_warmup_secs: 300,
            is_enabled: true,
            automations: vec![],
            worker_groups: vec![],
            error: None,
        };

        assert!(
            template_with_selections.is_automation_selected(&10),
            "Expected automation_id 10 to be selected"
        );
        assert!(
            !template_with_selections.is_automation_selected(&99),
            "Expected automation_id 99 not to be selected"
        );

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
            wol_enabled: false,
            wol_warmup_secs: 0,
            is_enabled: true,
            automations: vec![],
            worker_groups: vec![],
            error: None,
        };

        assert!(
            !template_without_selections.is_automation_selected(&10),
            "Expected false when automation_id is None"
        );
        assert!(
            !template_without_selections.is_worker_group_selected(&5),
            "Expected false when worker_group_id is None"
        );
    }

    #[test]
    fn test_schedule_form_template_accessibility_associations() {
        let user = AuthUser {
            id: 1,
            username: "editor".to_string(),
            display_name: "Editor User".to_string(),
            role: crate::auth::UserRole::Editor,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "csrf_token_test".to_string(),
            must_change_password: false,
        };

        let template = ScheduleFormTemplate {
            user,
            is_edit: false,
            schedule_id: None,
            automation_id: None,
            name: "Test Schedule".to_string(),
            cron_expression: "0 9 * * *".to_string(),
            timezone: "UTC".to_string(),
            worker_group_id: None,
            overlap_policy: "allow".to_string(),
            max_queue_age_secs: Some(3600),
            wol_enabled: true,
            wol_warmup_secs: 300,
            is_enabled: true,
            automations: vec![],
            worker_groups: vec![],
            error: None,
        };

        let rendered = template
            .render()
            .expect("ScheduleFormTemplate should render successfully");

        assert!(
            rendered.contains("aria-describedby=\"cron-expression-help\""),
            "Expected cron_expression input to have aria-describedby='cron-expression-help'"
        );
        assert!(
            rendered.contains("id=\"cron-expression-help\""),
            "Expected cron expression helper element to have id='cron-expression-help'"
        );

        assert!(
            rendered.contains("aria-describedby=\"overlap-policy-help\""),
            "Expected overlap_policy select to have aria-describedby='overlap-policy-help'"
        );
        assert!(
            rendered.contains("id=\"overlap-policy-help\""),
            "Expected overlap policy helper element to have id='overlap-policy-help'"
        );

        assert!(
            rendered.contains("aria-describedby=\"max-queue-age-help\""),
            "Expected max_queue_age_secs input to have aria-describedby='max-queue-age-help'"
        );
        assert!(
            rendered.contains("id=\"max-queue-age-help\""),
            "Expected max queue age helper element to have id='max-queue-age-help'"
        );

        assert!(
            rendered.contains("aria-describedby=\"wol-warmup-help\""),
            "Expected wol_warmup_secs input to have aria-describedby='wol-warmup-help'"
        );
        assert!(
            rendered.contains("id=\"wol-warmup-help\""),
            "Expected WOL warm-up helper element to have id='wol-warmup-help'"
        );
    }

    #[test]
    fn test_compute_next_run_at_expressions_and_edge_cases() {
        use chrono::TimeZone;

        let base_dt = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let res = compute_next_run_at("*/15 9-17 1,15 JAN-JUN MON-FRI", "UTC", &base_dt);
        assert!(res.is_ok());

        assert!(compute_next_run_at("invalid cron", "UTC", &base_dt).is_err());
        assert!(compute_next_run_at("60 * * * *", "UTC", &base_dt).is_err());
        assert!(compute_next_run_at("* 24 * * *", "UTC", &base_dt).is_err());

        let dec_31 = Utc.with_ymd_and_hms(2025, 12, 31, 23, 30, 0).unwrap();
        let next_year_jan1 = compute_next_run_at("0 0 1 1 *", "UTC", &dec_31).unwrap();
        assert_eq!(
            next_year_jan1,
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_month_end_transition() {
        use chrono::TimeZone;
        // Jan 31st 2025 23:55:00 UTC
        let jan_31 = Utc.with_ymd_and_hms(2025, 1, 31, 23, 55, 0).unwrap();
        // Daily schedule at midnight in Feb
        let res = compute_next_run_at("0 0 * FEB *", "UTC", &jan_31);
        assert!(res.is_ok());
        assert_eq!(
            res.unwrap(),
            Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_5_field_cron_second_truncation() {
        use chrono::TimeZone;
        // 11:59:30 UTC
        let t = Utc.with_ymd_and_hms(2025, 1, 15, 11, 59, 30).unwrap();
        // Schedule: top of every hour
        let res = compute_next_run_at("0 * * * *", "UTC", &t);
        assert!(res.is_ok());
        assert_eq!(
            res.unwrap(),
            Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_dow_range_parsing() {
        use chrono::TimeZone;
        let base_dt = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert!(compute_next_run_at("0 0 * * 1-7", "UTC", &base_dt).is_ok());
        assert!(compute_next_run_at("0 0 * * 5-7", "UTC", &base_dt).is_ok());
    }

    #[test]
    fn test_dom_dow_or_semantics() {
        use chrono::TimeZone;
        // Wed Jan 1, 2025
        let base_dt = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        // Cron: Midnight on 15th OR Mondays
        // First Monday is Jan 6, 2025
        let res = compute_next_run_at("0 0 15 * MON", "UTC", &base_dt).unwrap();
        assert_eq!(res, Utc.with_ymd_and_hms(2025, 1, 6, 0, 0, 0).unwrap());
    }
}
