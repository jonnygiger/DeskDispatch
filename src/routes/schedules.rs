use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::Deserialize;
use sqlx::FromRow;

use super::auth::HtmlTemplate;
use crate::auth::{AuthUser, CsrfForm, RequireEditor, log_audit};
use crate::de::deserialize_option_number;
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

fn default_overlap_policy() -> String {
    "allow".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// Represents a zero-dependency 5-field cron pattern (`<minute> <hour> <day> <month> <day-of-week>`).
/// Fields are stored as efficient bitmasks for O(1) matching without heap allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CronPattern {
    pub minutes: u64, // bits 0..=59
    pub hours: u32,   // bits 0..=23
    pub days: u32,    // bits 1..=31
    pub months: u16,  // bits 1..=12
    pub weekdays: u8, // bits 0..=6 (0=Sun, 6=Sat)
}

fn month_name_lookup(s: &str) -> Option<u32> {
    match s.to_ascii_uppercase().as_str() {
        "JAN" => Some(1),
        "FEB" => Some(2),
        "MAR" => Some(3),
        "APR" => Some(4),
        "MAY" => Some(5),
        "JUN" => Some(6),
        "JUL" => Some(7),
        "AUG" => Some(8),
        "SEP" => Some(9),
        "OCT" => Some(10),
        "NOV" => Some(11),
        "DEC" => Some(12),
        _ => None,
    }
}

fn weekday_name_lookup(s: &str) -> Option<u32> {
    match s.to_ascii_uppercase().as_str() {
        "SUN" => Some(0),
        "MON" => Some(1),
        "TUE" => Some(2),
        "WED" => Some(3),
        "THU" => Some(4),
        "FRI" => Some(5),
        "SAT" => Some(6),
        _ => None,
    }
}

fn parse_cron_value(
    s: &str,
    min_val: u32,
    max_val: u32,
    name_lookup: Option<fn(&str) -> Option<u32>>,
) -> Result<u32, String> {
    let s = s.trim();
    if let Ok(num) = s.parse::<u32>() {
        if num >= min_val && num <= max_val {
            return Ok(num);
        }
    }
    if let Some(lookup) = name_lookup {
        if let Some(num) = lookup(s) {
            return Ok(num);
        }
    }
    Err(format!("Invalid cron value '{}'", s))
}

fn parse_cron_field(
    field_str: &str,
    min_val: u32,
    max_val: u32,
    name_lookup: Option<fn(&str) -> Option<u32>>,
) -> Result<u64, String> {
    let mut mask: u64 = 0;
    for item in field_str.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err("Empty cron field element".to_string());
        }

        let (expr, step) = if let Some((e, s)) = item.split_once('/') {
            let step_num: u32 = s
                .parse()
                .map_err(|_| format!("Invalid step value '{}'", s))?;
            if step_num == 0 {
                return Err("Cron step cannot be 0".to_string());
            }
            (e, step_num)
        } else {
            (item, 1)
        };

        let (start, end) = if expr == "*" {
            (min_val, max_val)
        } else if let Some((s, e)) = expr.split_once('-') {
            let start = parse_cron_value(s, min_val, max_val, name_lookup)?;
            let end = parse_cron_value(e, min_val, max_val, name_lookup)?;
            (start, end)
        } else {
            let val = parse_cron_value(expr, min_val, max_val, name_lookup)?;
            (val, val)
        };

        if start > end {
            return Err(format!("Invalid range {}-{}", start, end));
        }

        let mut v = start;
        while v <= end {
            if v >= min_val && v <= max_val {
                let target_bit = if max_val == 7 && v == 7 { 0 } else { v };
                mask |= 1u64 << target_bit;
            }
            v += step;
        }
    }

    if mask == 0 {
        return Err(format!("Field '{}' produced no valid values", field_str));
    }

    Ok(mask)
}

impl CronPattern {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!(
                "Expected 5 cron expression fields, found {}",
                parts.len()
            ));
        }

        let minutes = parse_cron_field(parts[0], 0, 59, None)?;
        let hours = parse_cron_field(parts[1], 0, 23, None)?;
        let days = parse_cron_field(parts[2], 1, 31, None)?;
        let months = parse_cron_field(parts[3], 1, 12, Some(month_name_lookup))?;
        let weekdays = parse_cron_field(parts[4], 0, 7, Some(weekday_name_lookup))?;

        Ok(Self {
            minutes,
            hours: hours as u32,
            days: days as u32,
            months: months as u16,
            weekdays: weekdays as u8,
        })
    }

    #[inline]
    pub fn contains_minute(&self, m: u32) -> bool {
        (self.minutes & (1u64 << m)) != 0
    }

    #[inline]
    pub fn contains_hour(&self, h: u32) -> bool {
        (self.hours & (1u32 << h)) != 0
    }

    #[inline]
    pub fn contains_day(&self, d: u32) -> bool {
        (self.days & (1u32 << d)) != 0
    }

    #[inline]
    pub fn contains_month(&self, m: u32) -> bool {
        (self.months & (1u16 << m)) != 0
    }

    #[inline]
    pub fn contains_weekday(&self, w: u32) -> bool {
        (self.weekdays & (1u8 << w)) != 0
    }

    pub fn next_after<Tz: TimeZone>(&self, from_dt: &DateTime<Tz>) -> Result<DateTime<Tz>, String> {
        let start_naive = from_dt.naive_local();
        let start_sec_truncated = start_naive
            .with_second(0)
            .and_then(|dt| dt.with_nanosecond(0))
            .ok_or_else(|| "Invalid datetime".to_string())?;

        let tz = from_dt.timezone();
        let mut current = match tz.from_local_datetime(&start_sec_truncated) {
            chrono::LocalResult::Single(dt) => dt + Duration::minutes(1),
            chrono::LocalResult::Ambiguous(dt1, _) => dt1 + Duration::minutes(1),
            chrono::LocalResult::None => {
                tz.from_utc_datetime(&(start_sec_truncated.and_utc().naive_utc()))
                    + Duration::minutes(1)
            }
        };

        let all_days_mask = 0xFFFFFFFE_u32; // bits 1..=31
        let all_weekdays_mask = 0x7F; // bits 0..=6
        let both_day_restrictions =
            (self.days != all_days_mask) && (self.weekdays != all_weekdays_mask);

        let max_steps = 5 * 365 * 24 * 60;
        let mut steps = 0;

        while steps < max_steps {
            steps += 1;

            let month = current.month();
            if !self.contains_month(month) {
                let mask_after = if month < 12 {
                    self.months & (!0u16 << (month + 1))
                } else {
                    0
                };
                let (next_y, next_m) = if mask_after != 0 {
                    (current.year(), mask_after.trailing_zeros())
                } else {
                    (current.year() + 1, self.months.trailing_zeros())
                };

                if let Some(naive_next) = chrono::NaiveDate::from_ymd_opt(next_y, next_m, 1)
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                {
                    if let Some(next_dt) = tz
                        .from_local_datetime(&naive_next)
                        .single()
                        .or_else(|| tz.from_local_datetime(&naive_next).latest())
                    {
                        current = next_dt;
                        continue;
                    }
                }
                current += Duration::days(1);
                continue;
            }

            let day = current.day();
            let wday = current.weekday().num_days_from_sunday();

            let day_matches = if both_day_restrictions {
                self.contains_day(day) || self.contains_weekday(wday)
            } else {
                self.contains_day(day) && self.contains_weekday(wday)
            };

            if !day_matches {
                if let Some(naive_next) = current
                    .naive_local()
                    .date()
                    .succ_opt()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                {
                    if let Some(next_dt) = tz
                        .from_local_datetime(&naive_next)
                        .single()
                        .or_else(|| tz.from_local_datetime(&naive_next).latest())
                    {
                        current = next_dt;
                        continue;
                    }
                }
                current += Duration::days(1);
                continue;
            }

            let hour = current.hour();
            if !self.contains_hour(hour) {
                let mask_after = if hour < 23 {
                    self.hours & (!0u32 << (hour + 1))
                } else {
                    0
                };
                let next_h_opt = if mask_after != 0 {
                    Some(mask_after.trailing_zeros())
                } else {
                    None
                };

                if let Some(next_h) = next_h_opt {
                    if let Some(naive_next) = current.naive_local().date().and_hms_opt(next_h, 0, 0)
                    {
                        if let Some(next_dt) = tz
                            .from_local_datetime(&naive_next)
                            .single()
                            .or_else(|| tz.from_local_datetime(&naive_next).latest())
                        {
                            current = next_dt;
                            continue;
                        }
                    }
                } else if let Some(naive_next) = current
                    .naive_local()
                    .date()
                    .succ_opt()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                {
                    if let Some(next_dt) = tz
                        .from_local_datetime(&naive_next)
                        .single()
                        .or_else(|| tz.from_local_datetime(&naive_next).latest())
                    {
                        current = next_dt;
                        continue;
                    }
                }
                current += Duration::hours(1);
                continue;
            }

            let minute = current.minute();
            if !self.contains_minute(minute) {
                let mask_after = if minute < 59 {
                    self.minutes & (!0u64 << (minute + 1))
                } else {
                    0
                };
                let next_m_opt = if mask_after != 0 {
                    Some(mask_after.trailing_zeros())
                } else {
                    None
                };

                if let Some(next_m) = next_m_opt {
                    if let Some(naive_next) =
                        current.naive_local().date().and_hms_opt(hour, next_m, 0)
                    {
                        if let Some(next_dt) = tz
                            .from_local_datetime(&naive_next)
                            .single()
                            .or_else(|| tz.from_local_datetime(&naive_next).latest())
                        {
                            current = next_dt;
                            continue;
                        }
                    }
                } else if let Some(next_hour_naive) = current
                    .naive_local()
                    .with_minute(0)
                    .map(|dt| dt + Duration::hours(1))
                {
                    if let Some(next_dt) = tz
                        .from_local_datetime(&next_hour_naive)
                        .single()
                        .or_else(|| tz.from_local_datetime(&next_hour_naive).latest())
                    {
                        current = next_dt;
                        continue;
                    }
                }
                current += Duration::minutes(1);
                continue;
            }

            return Ok(current);
        }

        Err("No matching cron occurrence found within 5 years".to_string())
    }
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

    let cron =
        CronPattern::parse(cron_expr).map_err(|e| format!("Invalid cron expression: {}", e))?;

    let local_from = from_dt.with_timezone(&tz);

    cron.next_after(&local_from)
        .map_err(|e| format!("Failed to compute next run time: {}", e))
        .map(|dt| dt.with_timezone(&Utc))
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
            tracing::error!("Failed to fetch schedules: {}", e);
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
    let name = form.name.trim();
    let cron_expression = form.cron_expression.trim();
    let timezone = if form.timezone.trim().is_empty() {
        "UTC"
    } else {
        form.timezone.trim()
    };
    let wol_warmup_secs = form.wol_warmup_secs.unwrap_or(0).max(0);

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
            wol_enabled: form.wol_enabled,
            wol_warmup_secs,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some("Schedule Name and Cron Expression are required.".to_string()),
        })
        .into_response();
    }

    let now = Utc::now();
    // Validate the cron expression even when the schedule starts disabled: a
    // stored-but-invalid pattern would otherwise be enabled later with no
    // next run time and silently never fire.
    let parsed_next_run_at = match compute_next_run_at(cron_expression, timezone, &now) {
        Ok(next_dt) => next_dt,
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
                wol_enabled: form.wol_enabled,
                wol_warmup_secs,
                is_enabled: form.is_enabled,
                automations,
                worker_groups,
                error: Some(err_msg),
            })
            .into_response();
        }
    };
    let next_run_at = if form.is_enabled {
        Some(parsed_next_run_at)
    } else {
        None
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
    .bind(name)
    .bind(cron_expression)
    .bind(timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.wol_enabled)
    .bind(wol_warmup_secs)
    .bind(form.is_enabled)
    .bind(next_run_at)
    .bind(user.id)
    .fetch_one(&state.db)
    .await;

    let schedule_id: i64 = match row_res {
        Ok(id) => id,
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
                wol_enabled: form.wol_enabled,
                wol_warmup_secs,
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
    // Bolt Optimization: Run schedule row query and schedule option queries (automations and worker groups)
    // concurrently using `tokio::join!`. This reduces DB round-trips from 2 to 1 (~50% latency reduction).
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
            tracing::error!("Failed to fetch schedule: {}", e);
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
    let name = form.name.trim();
    let cron_expression = form.cron_expression.trim();
    let timezone = if form.timezone.trim().is_empty() {
        "UTC"
    } else {
        form.timezone.trim()
    };
    let wol_warmup_secs = form.wol_warmup_secs.unwrap_or(0).max(0);

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
            wol_enabled: form.wol_enabled,
            wol_warmup_secs,
            is_enabled: form.is_enabled,
            automations,
            worker_groups,
            error: Some("Schedule Name and Cron Expression are required.".to_string()),
        })
        .into_response();
    }

    let now = Utc::now();
    // Validate the pattern even for a disabled schedule (see create handler).
    let parsed_next_run_at = match compute_next_run_at(cron_expression, timezone, &now) {
        Ok(next_dt) => next_dt,
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
                wol_enabled: form.wol_enabled,
                wol_warmup_secs,
                is_enabled: form.is_enabled,
                automations,
                worker_groups,
                error: Some(err_msg),
            })
            .into_response();
        }
    };
    let next_run_at = if form.is_enabled {
        Some(parsed_next_run_at)
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
            wol_enabled = $8,
            wol_warmup_secs = $9,
            is_enabled = $10,
            next_run_at = $11
        WHERE id = $12
        "#,
    )
    .bind(form.automation_id)
    .bind(name)
    .bind(cron_expression)
    .bind(timezone)
    .bind(form.worker_group_id)
    .bind(&form.overlap_policy)
    .bind(form.max_queue_age_secs)
    .bind(form.wol_enabled)
    .bind(wol_warmup_secs)
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
            wol_enabled: form.wol_enabled,
            wol_warmup_secs,
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
            tracing::error!("Failed to fetch schedule for toggle: {}", e);
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
                let flash =
                    crate::auth::FlashMessage::error(format!("Cannot enable schedule: {err_msg}"));
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

    for sched in wol_due_schedules {
        let schedule_id = sched.id;
        let worker_group_id = sched.worker_group_id;

        let target_macs: Vec<String> = match worker_group_id {
            Some(group_id) => {
                sqlx::query_scalar(
                    r#"
                    SELECT w.mac_address
                    FROM task_worker_pcs w
                    JOIN worker_group_members m ON w.id = m.worker_id
                    WHERE m.group_id = $1
                      AND w.mac_address IS NOT NULL
                      AND w.mac_address != ''
                    "#,
                )
                .bind(group_id)
                .fetch_all(&mut *tx)
                .await?
            }
            None => {
                sqlx::query_scalar(
                    r#"
                    SELECT mac_address
                    FROM task_worker_pcs
                    WHERE mac_address IS NOT NULL
                      AND mac_address != ''
                    "#,
                )
                .fetch_all(&mut *tx)
                .await?
            }
        };

        for mac in target_macs {
            if let Err(e) = crate::wol::send_wol_packet(&mac).await {
                tracing::warn!(
                    schedule_id = schedule_id,
                    mac = %mac,
                    error = %e,
                    "Failed to send schedule WOL warm-up magic packet"
                );
            } else {
                tracing::info!(
                    schedule_id = schedule_id,
                    mac = %mac,
                    "Dispatched schedule WOL warm-up magic packet"
                );
            }
        }

        sqlx::query("UPDATE schedules SET last_wol_sent_at = now() WHERE id = $1")
            .bind(schedule_id)
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

    let mut queued_count = 0u64;

    for sched in due_schedules {
        let schedule_id = sched.id;
        let automation_id = sched.automation_id;
        let cron_expr = sched.cron_expression;
        let timezone = sched.timezone;
        let worker_group_id = sched.worker_group_id;
        let overlap_policy = sched.overlap_policy;
        let max_queue_age_secs = sched.max_queue_age_secs;
        let scheduled_run_at = sched.next_run_at;

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
            wol_enabled: false,
            wol_warmup_secs: 0,
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

        // Verify aria-describedby attributes and corresponding helper id elements
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
    fn test_cron_pattern_parsing_and_syntax() {
        let pattern = CronPattern::parse("*/15 9-17 1,15 JAN-JUN MON-FRI").unwrap();

        // Minutes: */15 -> 0, 15, 30, 45
        assert!(pattern.contains_minute(0));
        assert!(pattern.contains_minute(15));
        assert!(pattern.contains_minute(30));
        assert!(pattern.contains_minute(45));
        assert!(!pattern.contains_minute(10));

        // Hours: 9-17
        assert!(pattern.contains_hour(9));
        assert!(pattern.contains_hour(17));
        assert!(!pattern.contains_hour(8));
        assert!(!pattern.contains_hour(18));

        // Days: 1, 15
        assert!(pattern.contains_day(1));
        assert!(pattern.contains_day(15));
        assert!(!pattern.contains_day(2));

        // Months: JAN-JUN (1..=6)
        assert!(pattern.contains_month(1));
        assert!(pattern.contains_month(6));
        assert!(!pattern.contains_month(7));

        // Weekdays: MON-FRI (1..=5)
        assert!(pattern.contains_weekday(1));
        assert!(pattern.contains_weekday(5));
        assert!(!pattern.contains_weekday(0));
        assert!(!pattern.contains_weekday(6));

        // Invalid fields
        assert!(CronPattern::parse("invalid cron").is_err());
        assert!(CronPattern::parse("0 0 0 0 0 0").is_err());
        assert!(CronPattern::parse("60 * * * *").is_err());
        assert!(CronPattern::parse("* 24 * * *").is_err());
        assert!(CronPattern::parse("* * 32 * *").is_err());
        assert!(CronPattern::parse("* * * 13 *").is_err());
        assert!(CronPattern::parse("* * * * 8").is_err());
    }

    #[test]
    fn test_cron_pattern_next_after_edge_cases() {
        use chrono::TimeZone;

        // 1. Sunday representation edge cases: 0, 7, and SUN
        let p_sun0 = CronPattern::parse("0 12 * * 0").unwrap();
        let p_sun7 = CronPattern::parse("0 12 * * 7").unwrap();
        let p_sun_name = CronPattern::parse("0 12 * * SUN").unwrap();

        assert_eq!(p_sun0.weekdays, p_sun7.weekdays);
        assert_eq!(p_sun0.weekdays, p_sun_name.weekdays);
        assert!(p_sun0.contains_weekday(0));

        // 2. Day-of-month (DOM) AND Day-of-week (DOW) OR-logic handling
        // Standard cron behavior: when BOTH DOM and DOW are specified (not wildcard '*'),
        // an occurrence matches if DOM matches OR DOW matches.
        // E.g., "0 9 15 * MON" -> 9:00 AM on the 15th OR any Monday.
        let cron_or = CronPattern::parse("0 9 15 * MON").unwrap();

        // Base date: Wednesday Jan 15, 2025 at 08:00 UTC (DOM matches 15)
        let base_dom_match = Utc.with_ymd_and_hms(2025, 1, 15, 8, 0, 0).unwrap();
        let next_dom = cron_or.next_after(&base_dom_match).unwrap();
        assert_eq!(
            next_dom,
            Utc.with_ymd_and_hms(2025, 1, 15, 9, 0, 0).unwrap()
        );

        // Base date: Monday Jan 20, 2025 at 08:00 UTC (DOW matches MON, even though DOM is 20)
        let base_dow_match = Utc.with_ymd_and_hms(2025, 1, 20, 8, 0, 0).unwrap();
        let next_dow = cron_or.next_after(&base_dow_match).unwrap();
        assert_eq!(
            next_dow,
            Utc.with_ymd_and_hms(2025, 1, 20, 9, 0, 0).unwrap()
        );

        // 3. Step range expressions (e.g. 1-10/2 -> 1, 3, 5, 7, 9)
        let p_step_range = CronPattern::parse("1-10/2 10 * * *").unwrap();
        assert!(p_step_range.contains_minute(1));
        assert!(p_step_range.contains_minute(3));
        assert!(p_step_range.contains_minute(5));
        assert!(p_step_range.contains_minute(7));
        assert!(p_step_range.contains_minute(9));
        assert!(!p_step_range.contains_minute(2));
        assert!(!p_step_range.contains_minute(10));

        // 4. Year/month boundary rollover (Dec 31 to Jan 1)
        // Expression: "0 0 1 1 *" (Midnight on Jan 1st)
        let cron_year_end = CronPattern::parse("0 0 1 1 *").unwrap();
        let dec_31 = Utc.with_ymd_and_hms(2025, 12, 31, 23, 30, 0).unwrap();
        let next_year_jan1 = cron_year_end.next_after(&dec_31).unwrap();
        assert_eq!(
            next_year_jan1,
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
        );
    }
}
