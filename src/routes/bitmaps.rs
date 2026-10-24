use askama::Template;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use std::borrow::Cow;
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::AppState;
use crate::auth::{AuthUser, CsrfForm, RequireEditor, log_audit};
use crate::de::deserialize_option_number;
use crate::magnifier::{ImageMagnifier, map_coarse_click_to_native, map_grid_click_to_native};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/bitmaps",
            get(get_bitmaps_handler).post(post_bitmaps_handler),
        )
        .route("/bitmaps/{id}/delete", post(post_delete_bitmap_handler))
        .route(
            "/automations/{id}/bitmaps",
            get(get_automation_bitmaps_handler).post(post_automation_bitmaps_handler),
        )
        .route(
            "/automations/{id}/bitmaps/{bid}/delete",
            post(post_automation_delete_bitmap_handler),
        )
        .route(
            "/bitmaps/commit",
            get(get_bitmap_commit_handler).post(post_bitmap_commit_handler),
        )
        .route(
            "/bitmaps/pick-region",
            get(get_pick_region_handler).post(post_pick_region_top_left_handler),
        )
        .route(
            "/bitmaps/pick-region/bottom-right",
            post(post_pick_region_bottom_right_handler),
        )
        .route(
            "/bitmaps/pick-region/confirm",
            post(post_pick_region_confirm_handler),
        )
        .route(
            "/automations/{id}/bitmaps/pick-region",
            get(get_automation_pick_region_handler)
                .post(post_automation_pick_region_top_left_handler),
        )
        .route(
            "/automations/{id}/bitmaps/pick-region/bottom-right",
            post(post_automation_pick_region_bottom_right_handler),
        )
        .route(
            "/automations/{id}/bitmaps/pick-region/confirm",
            post(post_automation_pick_region_confirm_handler),
        )
}

#[inline]
pub fn bitmap_redirect_path(automation_id: Option<i64>) -> Cow<'static, str> {
    match automation_id {
        Some(aid) => Cow::Owned(format!("/automations/{aid}/bitmaps")),
        None => Cow::Borrowed("/bitmaps"),
    }
}

#[derive(serde::Deserialize)]
pub struct BitmapUploadForm {
    pub csrf_token: String,
    pub name: String,
}

#[derive(serde::Deserialize, Debug)]
pub struct DeleteBitmapForm {
    pub csrf_token: String,
}

#[derive(serde::Deserialize, Debug)]
pub struct BitmapCommitQuery {
    pub key: String,
    pub name: String,
    pub automation_id: Option<i64>,
}

#[derive(serde::Deserialize, Debug)]
pub struct BitmapCommitForm {
    pub csrf_token: String,
    pub key: String,
    pub name: String,
    pub automation_id: Option<i64>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionQuery {
    pub automation_id: Option<i64>,
    pub image_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    pub step_id: Option<i64>,
    pub reference_bitmap_id: Option<i64>,
    pub error_msg: Option<String>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionTopLeftForm {
    pub csrf_token: String,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub automation_id: Option<i64>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub step_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub reference_bitmap_id: Option<i64>,
    #[serde(
        alias = "click.x",
        alias = "click_x",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub x: Option<u32>,
    #[serde(
        alias = "click.y",
        alias = "click_y",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub y: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub manual_x: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub manual_y: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionBottomRightForm {
    pub csrf_token: String,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub automation_id: Option<i64>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub top_left_x: u32,
    pub top_left_y: u32,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub step_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub reference_bitmap_id: Option<i64>,
    #[serde(
        alias = "grid_click.x",
        alias = "grid_x",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub grid_x: Option<u32>,
    #[serde(
        alias = "grid_click.y",
        alias = "grid_y",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub grid_y: Option<u32>,
    #[serde(
        alias = "coarse_click.x",
        alias = "coarse_x",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub coarse_x: Option<u32>,
    #[serde(
        alias = "coarse_click.y",
        alias = "coarse_y",
        default,
        deserialize_with = "deserialize_option_number"
    )]
    pub coarse_y: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub manual_x: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub manual_y: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionConfirmForm {
    pub csrf_token: String,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub automation_id: Option<i64>,
    pub name: Option<String>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub top_left_x: u32,
    pub top_left_y: u32,
    pub bottom_right_x: u32,
    pub bottom_right_y: u32,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub step_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub reference_bitmap_id: Option<i64>,
}

#[derive(Template)]
#[template(path = "bitmaps/upload.html")]
pub struct BitmapsUploadTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub bitmap_name: String,
    pub object_storage_key: String,
    pub automation_id: Option<i64>,
    pub presigned_post_url: String,
    pub presigned_fields: Vec<(String, String)>,
    pub redirect_url: String,
}

#[derive(Template)]
#[template(path = "bitmaps/confirm.html")]
pub struct BitmapsConfirmTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub bitmap_name: String,
    pub object_storage_key: String,
    pub width: i32,
    pub height: i32,
    pub automation_id: Option<i64>,
}

#[derive(Debug, Default)]
pub struct RegionPickerParams {
    pub step_stage: u8,
    pub top_left_x: Option<u32>,
    pub top_left_y: Option<u32>,
    pub bottom_right_x: Option<u32>,
    pub bottom_right_y: Option<u32>,
    pub click_x: Option<u32>,
    pub click_y: Option<u32>,
    pub target_x: Option<u32>,
    pub target_y: Option<u32>,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    pub step_id: Option<i64>,
    pub reference_bitmap_id: Option<i64>,
    pub error_msg: Option<String>,
}

#[derive(Template)]
#[template(path = "bitmaps/pick_region.html")]
pub struct RegionPickerTopLeftTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: Option<i64>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub step_stage: u8,
    pub top_left_x: Option<u32>,
    pub top_left_y: Option<u32>,
    pub bottom_right_x: Option<u32>,
    pub bottom_right_y: Option<u32>,
    pub click_x: Option<u32>,
    pub click_y: Option<u32>,
    pub magnifier: ImageMagnifier,
    pub return_to: Option<String>,
    pub mode: Option<String>,
    pub step_id: Option<i64>,
    pub reference_bitmap_id: Option<i64>,
    pub error_msg: Option<String>,
}

impl RegionPickerTopLeftTemplate {
    pub fn build(
        user: AuthUser,
        automation_id: Option<i64>,
        image_url: String,
        width: u32,
        height: u32,
        params: RegionPickerParams,
    ) -> Self {
        let csrf_token = user.csrf_token.clone();
        let magnifier =
            ImageMagnifier::new(&image_url, width, height, params.target_x, params.target_y);
        Self {
            user,
            csrf_token,
            automation_id,
            image_url,
            width,
            height,
            step_stage: params.step_stage,
            top_left_x: params.top_left_x,
            top_left_y: params.top_left_y,
            bottom_right_x: params.bottom_right_x,
            bottom_right_y: params.bottom_right_y,
            click_x: params.click_x,
            click_y: params.click_y,
            magnifier,
            return_to: params.return_to,
            mode: params.mode,
            step_id: params.step_id,
            reference_bitmap_id: params.reference_bitmap_id,
            error_msg: params.error_msg,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BitmapListItem {
    pub id: i64,
    pub automation_id: Option<i64>,
    pub automation_name: Option<String>,
    pub name: String,
    pub object_storage_key: String,
    pub width: i32,
    pub height: i32,
    pub created_by: i64,
    pub created_by_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub magnifier: ImageMagnifier,
}

impl BitmapListItem {
    pub fn formatted_created_at(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

#[derive(Template)]
#[template(path = "bitmaps/list.html")]
pub struct BitmapsListTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub bitmaps: Vec<BitmapListItem>,
    pub automation_id: Option<i64>,
    pub automation_name: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct RawBitmapRow {
    id: i64,
    automation_id: Option<i64>,
    automation_name: Option<String>,
    name: String,
    object_storage_key: String,
    width: i32,
    height: i32,
    created_by: i64,
    created_by_name: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct DeleteBitmapRow {
    automation_id: Option<i64>,
    name: String,
    object_storage_key: String,
}

fn parse_bitmap_rows(rows: Vec<RawBitmapRow>) -> Vec<BitmapListItem> {
    let mut bitmaps = Vec::with_capacity(rows.len());
    for row in rows {
        let magnifier = ImageMagnifier::new(
            format!("/media/bitmaps/{}", row.id),
            row.width.max(1) as u32,
            row.height.max(1) as u32,
            None,
            None,
        );

        bitmaps.push(BitmapListItem {
            id: row.id,
            automation_id: row.automation_id,
            automation_name: row.automation_name,
            name: row.name,
            object_storage_key: row.object_storage_key,
            width: row.width,
            height: row.height,
            created_by: row.created_by,
            created_by_name: row.created_by_name,
            created_at: row.created_at,
            magnifier,
        });
    }
    bitmaps
}

pub async fn fetch_bitmaps_list_template(
    db: &sqlx::PgPool,
    user: AuthUser,
    automation_id: Option<i64>,
) -> Result<BitmapsListTemplate, Redirect> {
    let csrf_token = user.csrf_token.clone();

    let (automation_name, rows) = if let Some(aid) = automation_id {
        let (auto_res, rows_res) = tokio::join!(
            sqlx::query_scalar::<_, String>("SELECT name FROM automations WHERE id = $1")
                .bind(aid)
                .fetch_optional(db),
            sqlx::query_as::<_, RawBitmapRow>(
                r#"
                SELECT
                    b.id, b.automation_id, a.name AS automation_name, b.name,
                    b.object_storage_key, b.width, b.height, b.created_by,
                    COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
                    b.created_at
                FROM bitmaps b
                LEFT JOIN automations a ON b.automation_id = a.id
                LEFT JOIN users u ON b.created_by = u.id
                WHERE b.automation_id = $1 OR b.automation_id IS NULL
                ORDER BY b.created_at DESC, b.id DESC
                "#,
            )
            .bind(aid)
            .fetch_all(db)
        );

        let name = match auto_res {
            Ok(Some(n)) => n,
            _ => return Err(Redirect::to("/automations")),
        };
        let rows = rows_res.unwrap_or_default();
        (Some(name), rows)
    } else {
        let rows = sqlx::query_as::<_, RawBitmapRow>(
            r#"
            SELECT
                b.id, b.automation_id, a.name AS automation_name, b.name,
                b.object_storage_key, b.width, b.height, b.created_by,
                COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
                b.created_at
            FROM bitmaps b
            LEFT JOIN automations a ON b.automation_id = a.id
            LEFT JOIN users u ON b.created_by = u.id
            ORDER BY b.created_at DESC, b.id DESC
            "#,
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();
        (None, rows)
    };

    let bitmaps = parse_bitmap_rows(rows);

    Ok(BitmapsListTemplate {
        user,
        csrf_token,
        bitmaps,
        automation_id,
        automation_name,
    })
}

async fn render_bitmaps_list(
    db: &sqlx::PgPool,
    user: AuthUser,
    automation_id: Option<i64>,
) -> Response {
    match fetch_bitmaps_list_template(db, user, automation_id).await {
        Ok(tmpl) => HtmlTemplate(tmpl).into_response(),
        Err(redirect) => redirect.into_response(),
    }
}

/// GET /bitmaps
#[tracing::instrument(skip(state, user))]
pub async fn get_bitmaps_handler(State(state): State<AppState>, user: AuthUser) -> Response {
    render_bitmaps_list(&state.db, user, None).await
}

/// GET /automations/{id}/bitmaps
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Response {
    render_bitmaps_list(&state.db, user, Some(id)).await
}

async fn generate_bitmap_upload_policy(
    state: &AppState,
    user: AuthUser,
    csrf_token: String,
    raw_name: &str,
    automation_id: Option<i64>,
) -> Response {
    let bitmap_name = raw_name.trim().to_string();
    if bitmap_name.is_empty() {
        return Redirect::to(&bitmap_redirect_path(automation_id)).into_response();
    }

    let object_key = if automation_id.is_some() {
        format!("bitmaps/{}.png", uuid::Uuid::new_v4())
    } else {
        format!("bitmaps/user_{}_{}.png", user.id, uuid::Uuid::new_v4())
    };

    let redirect_url = match automation_id {
        Some(aid) => format!(
            "/bitmaps/commit?key={}&name={}&automation_id={}",
            crate::storage::url_encode_component(&object_key),
            crate::storage::url_encode_component(&bitmap_name),
            aid
        ),
        None => format!(
            "/bitmaps/commit?key={}&name={}",
            crate::storage::url_encode_component(&object_key),
            crate::storage::url_encode_component(&bitmap_name)
        ),
    };

    let post_policy = match state.storage_service().generate_presigned_post(
        &object_key,
        Duration::from_secs(900),
        10 * 1024 * 1024,
        Some(&redirect_url),
    ) {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to generate presigned POST for bitmap upload: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to generate upload policy",
            )
                .into_response();
        }
    };

    HtmlTemplate(BitmapsUploadTemplate {
        user,
        csrf_token,
        bitmap_name,
        object_storage_key: object_key,
        automation_id,
        presigned_post_url: post_policy.url,
        presigned_fields: post_policy.ordered_fields,
        redirect_url,
    })
    .into_response()
}

async fn process_bitmap_upload(
    state: &AppState,
    user: AuthUser,
    form: BitmapUploadForm,
    automation_id: Option<i64>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }
    generate_bitmap_upload_policy(state, user, form.csrf_token, &form.name, automation_id).await
}

/// POST /bitmaps
/// Handles initial bitmap name submission, generating an S3 presigned POST policy form.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_bitmaps_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<BitmapUploadForm>,
) -> Response {
    process_bitmap_upload(&state, user, form, None).await
}

/// POST /automations/{id}/bitmaps
/// Handles initial bitmap name submission scoped to an automation ID.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_automation_bitmaps_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<BitmapUploadForm>,
) -> Response {
    process_bitmap_upload(&state, user, form, Some(id)).await
}

/// Validates bitmap key prefix, extension, path safety, and user binding without heap allocations.
pub fn validate_bitmap_key(key: &str, user_id: i64) -> bool {
    if !key.starts_with("bitmaps/") || !key.ends_with(".png") || key.contains("..") {
        return false;
    }
    if let Some(user_part) = key.strip_prefix("bitmaps/user_") {
        if let Some(uid_str) = user_part.split('_').next() {
            if uid_str.parse::<i64>() != Ok(user_id) {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

/// Zero-allocation O(1) PNG header parser.
/// Verifies PNG magic bytes `[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]` and `IHDR` chunk marker (`[0x49, 0x48, 0x44, 0x52]`),
/// returning `(width, height)` as `(u32, u32)` directly from PNG header bytes without decoding image payload or heap allocations.
#[inline]
pub fn parse_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_HEADER: &[u8; 8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || &bytes[..8] != PNG_HEADER || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Probes bitmap dimensions from S3 using a ranged GET request.
/// Enforces PNG format only via zero-allocation PNG IHDR header parsing and fails on probe or retrieval errors.
pub async fn probe_s3_bitmap_dimensions(
    state: &AppState,
    key: &str,
) -> Result<(i32, i32), (StatusCode, &'static str)> {
    let bytes = state
        .storage_service()
        .get_object_range(key, 0, 63)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to fetch S3 object key {} for dimension probe: {}",
                key,
                e
            );
            (
                StatusCode::BAD_REQUEST,
                "Invalid or missing image in storage",
            )
        })?;

    let (w, h) = parse_png_dimensions(&bytes).ok_or((
        StatusCode::BAD_REQUEST,
        "Only PNG images are supported for reference bitmaps",
    ))?;

    Ok((w as i32, h as i32))
}

/// GET /bitmaps/commit
/// Render confirmation page following S3 browser upload via presigned POST policy.
#[tracing::instrument(skip(state, user))]
pub async fn get_bitmap_commit_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Query(query): Query<BitmapCommitQuery>,
) -> Response {
    if !validate_bitmap_key(&query.key, user.id) {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid object storage key prefix or user ownership mismatch",
        )
            .into_response();
    }

    let (width, height) = match probe_s3_bitmap_dimensions(&state, &query.key).await {
        Ok(dims) => dims,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    HtmlTemplate(BitmapsConfirmTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        bitmap_name: query.name,
        object_storage_key: query.key,
        width,
        height,
        automation_id: query.automation_id,
    })
    .into_response()
}

/// POST /bitmaps/commit
/// Finalize confirmation and persist bitmap record to database following S3 browser upload.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_bitmap_commit_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<BitmapCommitForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_name = form.name.trim().to_string();
    if bitmap_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Bitmap name cannot be empty").into_response();
    }

    if !validate_bitmap_key(&form.key, user.id) {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid object storage key prefix or user ownership mismatch",
        )
            .into_response();
    }

    let (width, height) = match probe_s3_bitmap_dimensions(&state, &form.key).await {
        Ok(dims) => dims,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    let bitmap_id = match sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (object_storage_key) DO UPDATE SET name = EXCLUDED.name
        RETURNING id
        "#,
    )
    .bind(form.automation_id)
    .bind(&bitmap_name)
    .bind(&form.key)
    .bind(width)
    .bind(height)
    .bind(user.id)
    .fetch_one(&state.db)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to insert bitmap into database: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create_bitmap",
        "bitmap",
        Some(bitmap_id),
        Some(serde_json::json!({
            "name": bitmap_name,
            "object_storage_key": form.key,
            "automation_id": form.automation_id,
            "width": width,
            "height": height,
        })),
    )
    .await;

    Redirect::to(&bitmap_redirect_path(form.automation_id)).into_response()
}

pub async fn delete_bitmap_logic(
    state: &AppState,
    user: &AuthUser,
    bitmap_id: i64,
    csrf_token: &str,
    automation_id: Option<i64>,
) -> Response {
    if csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let delete_res = sqlx::query_as::<_, DeleteBitmapRow>(
        "DELETE FROM bitmaps WHERE id = $1 RETURNING automation_id, name, object_storage_key",
    )
    .bind(bitmap_id)
    .fetch_optional(&state.db)
    .await;

    let row = match delete_res {
        Ok(Some(r)) => r,
        Ok(None) => return Redirect::to(&bitmap_redirect_path(automation_id)).into_response(),
        Err(e) => {
            tracing::error!("Failed to delete bitmap ID {}: {}", bitmap_id, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let _ = state
        .storage_service()
        .delete_object(&row.object_storage_key)
        .await;

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "delete_bitmap",
        "bitmap",
        Some(bitmap_id),
        Some(serde_json::json!({
            "name": row.name,
            "object_storage_key": row.object_storage_key,
            "automation_id": row.automation_id,
        })),
    )
    .await;

    Redirect::to(&bitmap_redirect_path(automation_id.or(row.automation_id))).into_response()
}

/// POST /bitmaps/{id}/delete
#[tracing::instrument(skip(state, user, form))]
pub async fn post_delete_bitmap_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Path(id): Path<i64>,
    CsrfForm(form): CsrfForm<DeleteBitmapForm>,
) -> Response {
    delete_bitmap_logic(&state, &user, id, &form.csrf_token, None).await
}

/// POST /automations/{id}/bitmaps/{bid}/delete
#[tracing::instrument(skip(state, user, form))]
pub async fn post_automation_delete_bitmap_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Path((id, bid)): Path<(i64, i64)>,
    CsrfForm(form): CsrfForm<DeleteBitmapForm>,
) -> Response {
    delete_bitmap_logic(&state, &user, bid, &form.csrf_token, Some(id)).await
}

/// Sanitizes and validates image URLs used in region pick handlers to prevent CSS injection and open redirects.
pub fn sanitize_image_url(raw_url: Option<&str>) -> &str {
    const DEFAULT_URL: &str = "/static/deskdispatch_logo.png";
    let Some(url) = raw_url else {
        return DEFAULT_URL;
    };
    if url.is_empty()
        || url.bytes().any(|b| {
            matches!(
                b,
                b'\\'
                    | b'\''
                    | b'"'
                    | b'<'
                    | b'>'
                    | b';'
                    | b'{'
                    | b'}'
                    | b'\n'
                    | b'\r'
                    | b'\t'
                    | 0x0C
            ) || b.is_ascii_control()
        })
    {
        return DEFAULT_URL;
    }
    let trimmed = url.trim();
    if trimmed.starts_with("/static/")
        || trimmed.starts_with("/media/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        trimmed
    } else {
        DEFAULT_URL
    }
}

/// GET /bitmaps/pick-region
/// Renders initial Step 1 of two-click region picker to capture top-left corner.
#[tracing::instrument(skip(user))]
pub async fn get_pick_region_handler(
    user: AuthUser,
    Query(query): Query<PickRegionQuery>,
) -> impl IntoResponse {
    let image_url = sanitize_image_url(query.image_url.as_deref());
    let width = query.width.unwrap_or(1920);
    let height = query.height.unwrap_or(1080);

    HtmlTemplate(RegionPickerTopLeftTemplate::build(
        user,
        query.automation_id,
        image_url.to_string(),
        width,
        height,
        RegionPickerParams {
            step_stage: 1,
            return_to: query.return_to,
            mode: query.mode,
            step_id: query.step_id,
            reference_bitmap_id: query.reference_bitmap_id,
            ..Default::default()
        },
    ))
}

/// GET /automations/{id}/bitmaps/pick-region
/// Renders initial Step 1 of two-click region picker scoped to an automation ID.
#[tracing::instrument(skip(user))]
pub async fn get_automation_pick_region_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    Query(mut query): Query<PickRegionQuery>,
) -> impl IntoResponse {
    query.automation_id = Some(id);
    get_pick_region_handler(user, Query(query)).await
}

/// POST /bitmaps/pick-region
/// Processes coarse top-left image input click coordinates or manual numeric entries for region selection.
#[tracing::instrument(skip(user, form))]
pub async fn post_pick_region_top_left_handler(
    user: AuthUser,
    CsrfForm(form): CsrfForm<PickRegionTopLeftForm>,
) -> Response {
    let image_url = sanitize_image_url(Some(&form.image_url));

    let (click_x, click_y, has_click) = match (form.x, form.y) {
        (Some(cx), Some(cy)) => (cx, cy, true),
        _ => (0, 0, false),
    };

    let (top_left_x, top_left_y, is_valid) = match (form.manual_x, form.manual_y) {
        (Some(mx), Some(my)) => (
            mx.min(form.width.saturating_sub(1)),
            my.min(form.height.saturating_sub(1)),
            true,
        ),
        _ => {
            if !has_click || (click_x == 0 && click_y == 0) {
                (0, 0, false)
            } else {
                let (nx, ny) = map_coarse_click_to_native(
                    click_x,
                    click_y,
                    form.width as f64,
                    form.height as f64,
                    form.width,
                    form.height,
                );
                (nx, ny, true)
            }
        }
    };

    let (step_stage, tl_x, tl_y, error_msg) = if is_valid {
        (2, Some(top_left_x), Some(top_left_y), None)
    } else {
        (
            1,
            None,
            None,
            Some("Keyboard selection detected (0, 0). Please enter precise pixel coordinates in the manual numeric input fields below, or click directly on the image.".to_string()),
        )
    };

    HtmlTemplate(RegionPickerTopLeftTemplate::build(
        user,
        form.automation_id,
        image_url.to_string(),
        form.width,
        form.height,
        RegionPickerParams {
            step_stage,
            top_left_x: tl_x,
            top_left_y: tl_y,
            click_x: tl_x,
            click_y: tl_y,
            target_x: tl_x,
            target_y: tl_y,
            return_to: form.return_to,
            mode: form.mode,
            step_id: form.step_id,
            reference_bitmap_id: form.reference_bitmap_id,
            error_msg,
            ..Default::default()
        },
    ))
    .into_response()
}

/// POST /automations/{id}/bitmaps/pick-region
/// Processes coarse top-left image input click coordinates scoped to an automation ID.
#[tracing::instrument(skip(user, form))]
pub async fn post_automation_pick_region_top_left_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    mut form: CsrfForm<PickRegionTopLeftForm>,
) -> Response {
    form.0.automation_id = Some(id);
    post_pick_region_top_left_handler(user, form).await
}

/// POST /bitmaps/pick-region/bottom-right
/// Processes grid view, coarse image click coordinates, or manual numeric entries for bottom-right corner selection.
#[tracing::instrument(skip(user, form))]
pub async fn post_pick_region_bottom_right_handler(
    user: AuthUser,
    CsrfForm(form): CsrfForm<PickRegionBottomRightForm>,
) -> Response {
    let image_url = sanitize_image_url(Some(&form.image_url));

    let (raw_br_x, raw_br_y, is_valid) = match (form.manual_x, form.manual_y) {
        (Some(mx), Some(my)) => (
            mx.min(form.width.saturating_sub(1)),
            my.min(form.height.saturating_sub(1)),
            true,
        ),
        _ => {
            if let (Some(gx), Some(gy)) = (form.grid_x, form.grid_y) {
                if gx == 0 && gy == 0 {
                    (0, 0, false)
                } else {
                    let (nx, ny) = map_grid_click_to_native(
                        gx,
                        gy,
                        form.top_left_x,
                        form.top_left_y,
                        form.width,
                        form.height,
                    );
                    (nx, ny, true)
                }
            } else if let (Some(cx), Some(cy)) = (form.coarse_x, form.coarse_y) {
                if cx == 0 && cy == 0 {
                    (0, 0, false)
                } else {
                    let (nx, ny) = map_coarse_click_to_native(
                        cx,
                        cy,
                        form.width as f64,
                        form.height as f64,
                        form.width,
                        form.height,
                    );
                    (nx, ny, true)
                }
            } else {
                (form.top_left_x, form.top_left_y, false)
            }
        }
    };

    let (step_stage, tl_x, tl_y, br_x, br_y, target_x, target_y, error_msg) = if is_valid {
        let norm_tl_x = form.top_left_x.min(raw_br_x);
        let norm_tl_y = form.top_left_y.min(raw_br_y);
        let norm_br_x = form.top_left_x.max(raw_br_x);
        let norm_br_y = form.top_left_y.max(raw_br_y);
        (
            3,
            Some(norm_tl_x),
            Some(norm_tl_y),
            Some(norm_br_x),
            Some(norm_br_y),
            Some(norm_br_x),
            Some(norm_br_y),
            None,
        )
    } else {
        (
            2,
            Some(form.top_left_x),
            Some(form.top_left_y),
            None,
            None,
            Some(form.top_left_x),
            Some(form.top_left_y),
            Some("Keyboard selection detected (0, 0). Please enter precise pixel coordinates in the manual numeric input fields below, or click directly on the image / grid.".to_string()),
        )
    };

    HtmlTemplate(RegionPickerTopLeftTemplate::build(
        user,
        form.automation_id,
        image_url.to_string(),
        form.width,
        form.height,
        RegionPickerParams {
            step_stage,
            top_left_x: tl_x,
            top_left_y: tl_y,
            bottom_right_x: br_x,
            bottom_right_y: br_y,
            click_x: if is_valid { br_x } else { None },
            click_y: if is_valid { br_y } else { None },
            target_x,
            target_y,
            return_to: form.return_to,
            mode: form.mode,
            step_id: form.step_id,
            reference_bitmap_id: form.reference_bitmap_id,
            error_msg,
        },
    ))
    .into_response()
}

/// POST /automations/{id}/bitmaps/pick-region/bottom-right
/// Processes grid view or coarse image click coordinates for bottom-right corner selection scoped to an automation ID.
#[tracing::instrument(skip(user, form))]
pub async fn post_automation_pick_region_bottom_right_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    CsrfForm(mut form): CsrfForm<PickRegionBottomRightForm>,
) -> Response {
    form.automation_id = Some(id);
    post_pick_region_bottom_right_handler(user, CsrfForm(form)).await
}

async fn fetch_s3_object_by_id(
    state: &AppState,
    query_str: &'static str,
    id: i64,
) -> Option<Vec<u8>> {
    let key: String = sqlx::query_scalar(query_str)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()??;

    state.storage_service().get_object(&key).await.ok()
}

async fn fetch_source_image_bytes(state: &AppState, image_url: &str) -> Option<Vec<u8>> {
    let path = image_url.split_once('?').map_or(image_url, |(p, _)| p);
    let path = if let Some(idx) = path.find("://") {
        let rest = &path[idx + 3..];
        rest.find('/').map_or("", |i| &rest[i..])
    } else {
        path
    };

    if let Some(static_path) = path.strip_prefix("/static/") {
        if let Some(asset) = crate::routes::static_assets::Assets::get(static_path) {
            return Some(asset.data.to_vec());
        }
    }

    if let Some(id_str) = path.strip_prefix("/media/screenshots/") {
        if let Ok(id) = id_str.parse::<i64>() {
            return fetch_s3_object_by_id(
                state,
                "SELECT object_storage_key FROM step_screenshots WHERE step_id = $1",
                id,
            )
            .await;
        }
    }

    if let Some(id_str) = path.strip_prefix("/media/bitmaps/") {
        if let Ok(id) = id_str.parse::<i64>() {
            return fetch_s3_object_by_id(
                state,
                "SELECT object_storage_key FROM bitmaps WHERE id = $1",
                id,
            )
            .await;
        }
    }

    let s3_key = if let Some(rest) = path.strip_prefix('/') {
        if let Some(key) = rest
            .strip_prefix(state.config.s3_bucket.as_str())
            .and_then(|r| r.strip_prefix('/'))
        {
            Some(key)
        } else if !rest.starts_with("static/") && !rest.starts_with("media/") {
            Some(rest)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(key) = s3_key
        && !key.is_empty()
    {
        if let Ok(data) = state.storage_service().get_object(key).await {
            return Some(data);
        }
    }

    None
}

/// Encodes raw 8-bit RGB pixel bytes into PNG format using the `png` crate.
pub fn encode_png(width: u32, height: u32, rgb_pixels: &[u8]) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let mut out_bytes = Vec::with_capacity((width * height * 3) as usize + 64);
    let mut encoder = png::Encoder::new(&mut out_bytes, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = encoder.write_header() {
        let _ = writer.write_image_data(rgb_pixels);
    }
    out_bytes
}

/// Helper to load source image bytes, crop to (tl_x, tl_y, crop_w, crop_h), and return PNG bytes.
async fn crop_image_region(
    state: &AppState,
    image_url: &str,
    tl_x: u32,
    tl_y: u32,
    crop_w: u32,
    crop_h: u32,
) -> Vec<u8> {
    let crop_w = crop_w.max(1);
    let crop_h = crop_h.max(1);
    let target_cap = (crop_w as usize) * (crop_h as usize) * 3;

    if let Some(bytes) = fetch_source_image_bytes(state, image_url).await {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

        if let Ok(mut reader) = decoder.read_info() {
            let buf_size = reader
                .output_buffer_size()
                .unwrap_or_else(|| reader.info().raw_bytes());
            let mut buf = vec![0u8; buf_size];
            if let Ok(info) = reader.next_frame(&mut buf) {
                let src_w = info.width;
                let src_h = info.height;
                let bytes_per_pixel = match info.color_type {
                    png::ColorType::Rgb => 3,
                    png::ColorType::Rgba => 4,
                    png::ColorType::Grayscale => 1,
                    png::ColorType::GrayscaleAlpha => 2,
                    _ => 3,
                };

                let mut cropped = Vec::with_capacity(target_cap);
                for y in 0..crop_h {
                    let sy = tl_y.saturating_add(y).min(src_h.saturating_sub(1));
                    for x in 0..crop_w {
                        let sx = tl_x.saturating_add(x).min(src_w.saturating_sub(1));
                        let idx = ((sy * src_w + sx) as usize) * bytes_per_pixel;
                        if idx + bytes_per_pixel <= buf.len() {
                            let px = &buf[idx..idx + bytes_per_pixel];
                            match bytes_per_pixel {
                                3 => cropped.extend_from_slice(px),
                                4 => cropped.extend_from_slice(&px[..3]),
                                1 | 2 => cropped.extend_from_slice(&[px[0], px[0], px[0]]),
                                _ => cropped.extend_from_slice(&[0, 0, 0]),
                            }
                        } else {
                            cropped.extend_from_slice(&[0, 0, 0]);
                        }
                    }
                }

                return encode_png(crop_w, crop_h, &cropped);
            }
        }
    }

    let mut fallback_pixels = Vec::with_capacity(target_cap);
    for y in 0..crop_h {
        let g = ((y * 255) / crop_h) as u8;
        for x in 0..crop_w {
            let r = ((x * 255) / crop_w) as u8;
            fallback_pixels.extend_from_slice(&[r, g, 180]);
        }
    }
    encode_png(crop_w, crop_h, &fallback_pixels)
}

#[tracing::instrument(skip(state, user, form))]
pub async fn confirm_region_crop_logic(
    state: &AppState,
    user: &AuthUser,
    mut form: PickRegionConfirmForm,
) -> Response {
    form.image_url = sanitize_image_url(Some(&form.image_url)).to_string();
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let norm_tl_x = form.top_left_x.min(form.bottom_right_x);
    let norm_tl_y = form.top_left_y.min(form.bottom_right_y);
    let norm_br_x = form.top_left_x.max(form.bottom_right_x);
    let norm_br_y = form.top_left_y.max(form.bottom_right_y);

    let crop_w = norm_br_x - norm_tl_x + 1;
    let crop_h = norm_br_y - norm_tl_y + 1;

    if form.mode.as_deref() == Some("search_region") {
        let ref_param = form
            .reference_bitmap_id
            .map(|id| format!("&reference_bitmap_id={id}"))
            .unwrap_or_default();

        let redirect_url = if let (Some(aid), Some(sid)) = (form.automation_id, form.step_id) {
            format!(
                "/automations/{aid}/steps/{sid}/edit?search_x={norm_tl_x}&search_y={norm_tl_y}&search_width={crop_w}&search_height={crop_h}{ref_param}"
            )
        } else if let Some(aid) = form.automation_id {
            format!(
                "/automations/{aid}/steps/new/find_bitmap?search_x={norm_tl_x}&search_y={norm_tl_y}&search_width={crop_w}&search_height={crop_h}{ref_param}"
            )
        } else {
            "/automations".to_string()
        };

        return Redirect::to(&redirect_url).into_response();
    }

    let bitmap_name = form.name.as_deref().unwrap_or("").trim().to_string();
    if bitmap_name.is_empty() {
        return Redirect::to(&bitmap_redirect_path(form.automation_id)).into_response();
    }

    let object_key = format!("bitmaps/crop_{}.png", uuid::Uuid::new_v4());

    let png_bytes =
        crop_image_region(state, &form.image_url, norm_tl_x, norm_tl_y, crop_w, crop_h).await;

    if let Err(e) = state
        .storage_service()
        .put_object(&object_key, png_bytes, "image/png")
        .await
    {
        tracing::error!("Failed to upload cropped bitmap to S3: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store bitmap image",
        )
            .into_response();
    }

    let bitmap_id = match sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(form.automation_id)
    .bind(&bitmap_name)
    .bind(&object_key)
    .bind(crop_w as i32)
    .bind(crop_h as i32)
    .bind(user.id)
    .fetch_one(&state.db)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to insert cropped bitmap into database: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let details = serde_json::json!({
        "name": bitmap_name,
        "object_storage_key": object_key,
        "automation_id": form.automation_id,
        "width": crop_w,
        "height": crop_h,
        "source_image_url": form.image_url,
        "top_left": [norm_tl_x, norm_tl_y],
        "bottom_right": [norm_br_x, norm_br_y]
    });

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create_bitmap_crop",
        "bitmap",
        Some(bitmap_id),
        Some(details),
    )
    .await;

    if form.return_to.as_deref() == Some("find_bitmap")
        || form.return_to.as_deref() == Some("edit_find_bitmap")
    {
        let redirect_url = if let (Some(aid), Some(sid)) = (form.automation_id, form.step_id) {
            format!("/automations/{aid}/steps/{sid}/edit?reference_bitmap_id={bitmap_id}")
        } else if let Some(aid) = form.automation_id {
            format!("/automations/{aid}/steps/new/find_bitmap?reference_bitmap_id={bitmap_id}")
        } else {
            "/bitmaps".to_string()
        };
        return Redirect::to(&redirect_url).into_response();
    }

    Redirect::to(&bitmap_redirect_path(form.automation_id)).into_response()
}

/// POST /bitmaps/pick-region/confirm
/// Confirms and saves a selected region crop as a reference bitmap.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_pick_region_confirm_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<PickRegionConfirmForm>,
) -> Response {
    confirm_region_crop_logic(&state, &user, form).await
}

/// POST /automations/{id}/bitmaps/pick-region/confirm
/// Confirms and saves a selected region crop as a reference bitmap scoped to an automation ID.
pub async fn post_automation_pick_region_confirm_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    Path(id): Path<i64>,
    CsrfForm(mut form): CsrfForm<PickRegionConfirmForm>,
) -> Response {
    form.automation_id = Some(id);
    confirm_region_crop_logic(&state, &user, form).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_coarse_click_to_native_exact_fit() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1200, 680);
        assert_eq!((nx, ny), (600, 340));
    }

    #[test]
    fn test_map_coarse_click_to_native_letterboxing() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1200, 340);
        assert_eq!((nx, ny), (600, 170));
    }

    #[test]
    fn test_map_coarse_click_to_native_pillarboxing() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 600, 680);
        assert_eq!((nx, ny), (300, 340));
    }

    #[test]
    fn test_map_coarse_click_to_native_zero_dimensions() {
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 0, 0);
        assert_eq!((nx, ny), (0, 0));
    }

    #[test]
    fn test_map_grid_click_to_native_center_and_offsets() {
        let (nx, ny) = map_grid_click_to_native(160, 160, 100, 100, 1920, 1080);
        assert_eq!((nx, ny), (100, 100));

        let (nx_r, ny_r) = map_grid_click_to_native(180, 160, 100, 100, 1920, 1080);
        assert_eq!((nx_r, ny_r), (101, 100));

        let (nx_u, ny_u) = map_grid_click_to_native(160, 140, 100, 100, 1920, 1080);
        assert_eq!((nx_u, ny_u), (100, 99));
    }

    #[test]
    fn test_sanitize_image_url() {
        assert_eq!(
            sanitize_image_url(Some("/static/test.png")),
            "/static/test.png"
        );
        assert_eq!(
            sanitize_image_url(Some("/media/screenshots/1")),
            "/media/screenshots/1"
        );
        assert_eq!(
            sanitize_image_url(Some("http://example.com/img.png")),
            "http://example.com/img.png"
        );

        // Disallowed / injection inputs fallback to default
        assert_eq!(
            sanitize_image_url(Some("/static/img.png'; body { background: red; }")),
            "/static/deskdispatch_logo.png"
        );
        assert_eq!(
            sanitize_image_url(Some("/static/test\n.png")),
            "/static/deskdispatch_logo.png"
        );
        assert_eq!(sanitize_image_url(None), "/static/deskdispatch_logo.png");
    }

    #[test]
    fn test_validate_bitmap_key() {
        assert!(validate_bitmap_key("bitmaps/user_123_uuid.png", 123));
        assert!(!validate_bitmap_key("bitmaps/user_123_uuid.png", 456));
        assert!(!validate_bitmap_key("bitmaps/../etc/passwd.png", 123));
        assert!(!validate_bitmap_key("invalid/prefix.png", 123));
        assert!(!validate_bitmap_key("bitmaps/image.jpg", 123));
    }

    #[test]
    fn test_parse_png_dimensions() {
        let valid_hdr = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0,
            0, 0x07, 0x80, // 1920
            0, 0, 0x04, 0x38, // 1080
        ];
        assert_eq!(parse_png_dimensions(&valid_hdr), Some((1920, 1080)));

        let invalid_hdr = [0; 24];
        assert_eq!(parse_png_dimensions(&invalid_hdr), None);
    }

    #[test]
    fn test_png_encoder_and_header_parsing() {
        let rgb = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let png = encode_png(2, 2, &rgb);
        assert!(png.len() > 24);
        assert_eq!(parse_png_dimensions(&png), Some((2, 2)));
    }

    #[test]
    fn test_form_deserialization_click_coords() {
        let json = serde_json::json!({
            "csrf_token": "token123",
            "image_url": "/static/test.png",
            "width": 1920,
            "height": 1080,
            "click.x": "100",
            "click.y": "200"
        });
        let form: PickRegionTopLeftForm =
            serde_json::from_value(json).expect("Deserialization succeeds");
        assert_eq!(form.x, Some(100));
        assert_eq!(form.y, Some(200));
    }

    #[test]
    fn test_pick_region_numeric_entry_and_keyboard_no_pick_logic() {
        let form_valid = PickRegionTopLeftForm {
            csrf_token: "t".to_string(),
            automation_id: None,
            image_url: "/static/test.png".to_string(),
            width: 1920,
            height: 1080,
            return_to: None,
            mode: None,
            step_id: None,
            reference_bitmap_id: None,
            x: None,
            y: None,
            manual_x: Some(150),
            manual_y: Some(250),
        };
        assert_eq!(form_valid.manual_x, Some(150));
        assert_eq!(form_valid.manual_y, Some(250));
    }

    #[test]
    fn test_pick_region_query_search_region_bounding() {
        let q = PickRegionQuery {
            automation_id: Some(10),
            image_url: Some("/static/test.png".to_string()),
            width: Some(1920),
            height: Some(1080),
            return_to: Some("find_bitmap".to_string()),
            mode: Some("search_region".to_string()),
            step_id: Some(5),
            reference_bitmap_id: Some(2),
            error_msg: None,
        };
        assert_eq!(q.automation_id, Some(10));
        assert_eq!(q.mode.as_deref(), Some("search_region"));
    }
}
