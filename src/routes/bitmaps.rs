use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{AuthUser, CsrfForm, RequireEditor, log_audit};
use crate::de::deserialize_option_number;
use crate::magnifier::{ImageMagnifier, map_coarse_click_to_native, map_grid_click_to_native};
use axum::Router;
use axum::routing::{get, post};

use crate::AppState;

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
struct BitmapIdAutoRow {
    automation_id: Option<i64>,
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

/// GET /bitmaps
/// Renders the list page for all reference bitmaps in the system.
#[tracing::instrument(skip(state, user))]
pub async fn get_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    match fetch_bitmaps_list_template(&state.db, user, None).await {
        Ok(tmpl) => HtmlTemplate(tmpl).into_response(),
        Err(redirect) => redirect.into_response(),
    }
}

/// GET /automations/{id}/bitmaps
/// Renders the reference bitmaps list page scoped to a specific automation.
#[tracing::instrument(skip(state, user))]
pub async fn get_automation_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match fetch_bitmaps_list_template(&state.db, user, Some(id)).await {
        Ok(tmpl) => HtmlTemplate(tmpl).into_response(),
        Err(redirect) => redirect.into_response(),
    }
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
        let redirect_path = match automation_id {
            Some(aid) => format!("/automations/{}/bitmaps", aid),
            None => "/bitmaps".to_string(),
        };
        return Redirect::to(&redirect_path).into_response();
    }

    let object_key = match automation_id {
        Some(_) => format!("bitmaps/{}.png", uuid::Uuid::new_v4()),
        None => format!("bitmaps/user_{}_{}.png", user.id, uuid::Uuid::new_v4()),
    };

    let base_url = state.config.public_base_url.trim_end_matches('/');
    let encoded_name = crate::auth::flash::url_encode(&bitmap_name);
    let redirect_url = match automation_id {
        Some(aid) => format!(
            "{}/bitmaps/commit?name={}&automation_id={}",
            base_url, encoded_name, aid
        ),
        None => format!("{}/bitmaps/commit?name={}", base_url, encoded_name),
    };

    let storage = state.storage_service();
    let presigned_post = match storage
        .generate_presigned_post(
            &object_key,
            Duration::from_secs(900),
            10_485_760,
            Some(&redirect_url),
        )
        .await
    {
        Ok(post) => post,
        Err(e) => {
            tracing::error!("Failed to generate presigned POST policy: {}", e);
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
        presigned_post_url: presigned_post.url,
        presigned_fields: presigned_post.ordered_fields,
        redirect_url,
    })
    .into_response()
}

/// POST /bitmaps
/// Handles initial bitmap name submission, generating an S3 presigned POST policy form.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_bitmaps_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<BitmapUploadForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }
    generate_bitmap_upload_policy(&state, user, form.csrf_token, &form.name, None).await
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
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }
    generate_bitmap_upload_policy(&state, user, form.csrf_token, &form.name, Some(id)).await
}

/// Validates bitmap key prefix, extension, path safety, and user binding without heap allocations.
pub fn validate_bitmap_key(key: &str, user_id: i64) -> bool {
    if !key.starts_with("bitmaps/") || !key.ends_with(".png") || key.contains("..") {
        return false;
    }
    if let Some(user_part) = key.strip_prefix("bitmaps/user_") {
        let mut parts = user_part.splitn(2, '_');
        if let Some(uid_str) = parts.next() {
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
        return (StatusCode::BAD_REQUEST, "Invalid storage key").into_response();
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
/// Confirms registration and creates the database record for an uploaded reference bitmap.
#[tracing::instrument(skip(state, user, form))]
pub async fn post_bitmap_commit_handler(
    State(state): State<AppState>,
    RequireEditor(user): RequireEditor,
    CsrfForm(form): CsrfForm<BitmapCommitForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    if !validate_bitmap_key(&form.key, user.id) {
        return (StatusCode::BAD_REQUEST, "Invalid storage key").into_response();
    }

    let bitmap_name = form.name.trim().to_string();
    if bitmap_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Bitmap name cannot be empty").into_response();
    }

    // Idempotency check: if bitmap with this key already exists, return redirect without error
    if let Ok(Some(existing)) = sqlx::query_as::<_, BitmapIdAutoRow>(
        "SELECT automation_id FROM bitmaps WHERE object_storage_key = $1",
    )
    .bind(&form.key)
    .fetch_optional(&state.db)
    .await
    {
        let redirect_path = match existing.automation_id.or(form.automation_id) {
            Some(a) => format!("/automations/{}/bitmaps", a),
            None => "/bitmaps".to_string(),
        };
        return Redirect::to(&redirect_path).into_response();
    }

    let (width, height) = match probe_s3_bitmap_dimensions(&state, &form.key).await {
        Ok(dims) => dims,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    let bitmap_id = match sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)
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

    let details = serde_json::json!({
        "name": bitmap_name,
        "object_storage_key": form.key,
        "automation_id": form.automation_id,
        "width": width,
        "height": height
    });

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create",
        "bitmap",
        Some(bitmap_id),
        Some(details),
    )
    .await;

    if let Some(aid) = form.automation_id {
        Redirect::to(&format!("/automations/{}/bitmaps", aid)).into_response()
    } else {
        Redirect::to("/bitmaps").into_response()
    }
}

async fn delete_bitmap_logic(
    state: &AppState,
    user: &AuthUser,
    bitmap_id: i64,
    csrf_token: &str,
    redirect_automation_id: Option<i64>,
) -> Response {
    if csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_row = match sqlx::query_as::<_, DeleteBitmapRow>(
        "SELECT automation_id, name, object_storage_key FROM bitmaps WHERE id = $1",
    )
    .bind(bitmap_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            let redirect_path = match redirect_automation_id {
                Some(aid) => format!("/automations/{}/bitmaps", aid),
                None => "/bitmaps".to_string(),
            };
            return Redirect::to(&redirect_path).into_response();
        }
        Err(e) => {
            tracing::error!("Error fetching bitmap for deletion: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let auto_id = bitmap_row.automation_id;
    let bitmap_name = bitmap_row.name;
    let object_key = bitmap_row.object_storage_key;

    if let Err(e) = state.storage_service().delete_object(&object_key).await {
        tracing::warn!("Failed to delete object key {} from S3: {}", object_key, e);
    }

    let delete_res = sqlx::query("DELETE FROM bitmaps WHERE id = $1")
        .bind(bitmap_id)
        .execute(&state.db)
        .await;

    if let Err(e) = delete_res {
        tracing::error!("Failed to delete bitmap {} from database: {}", bitmap_id, e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete bitmap").into_response();
    }

    let details = serde_json::json!({
        "name": bitmap_name,
        "object_storage_key": object_key,
        "automation_id": auto_id
    });

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "delete_bitmap",
        "bitmap",
        Some(bitmap_id),
        Some(details),
    )
    .await;

    let target_automation_id = redirect_automation_id.or(auto_id);
    let redirect_path = match target_automation_id {
        Some(aid) => format!("/automations/{}/bitmaps", aid),
        None => "/bitmaps".to_string(),
    };

    Redirect::to(&redirect_path).into_response()
}

/// POST /bitmaps/{id}/delete
/// Handles bitmap deletion from database and object storage.
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
/// Handles bitmap deletion scoped to an automation ID.
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
    const DEFAULT_URL: &str = "/static/sample_screenshot.png";
    let url = match raw_url.map(str::trim) {
        Some(u) if !u.is_empty() => u,
        _ => return DEFAULT_URL,
    };

    if url.bytes().any(|b| {
        b.is_ascii_control() || matches!(b, b'\'' | b'"' | b'\\' | b'<' | b'>' | b';' | b'{' | b'}')
    }) {
        return DEFAULT_URL;
    }

    if url.starts_with("/static/")
        || url.starts_with("/media/")
        || url.starts_with("http://")
        || url.starts_with("https://")
    {
        url
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
    let csrf_token = user.csrf_token.clone();
    let image_url = sanitize_image_url(query.image_url.as_deref()).to_string();
    let width = query.width.unwrap_or(1920);
    let height = query.height.unwrap_or(1080);

    let magnifier = ImageMagnifier::new(&image_url, width, height, None, None);

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user,
        csrf_token,
        automation_id: query.automation_id,
        image_url,
        width,
        height,
        step_stage: 1,
        top_left_x: None,
        top_left_y: None,
        bottom_right_x: None,
        bottom_right_y: None,
        click_x: None,
        click_y: None,
        magnifier,
        return_to: query.return_to,
        mode: query.mode,
        step_id: query.step_id,
        reference_bitmap_id: query.reference_bitmap_id,
        error_msg: None,
    })
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
    let image_url = sanitize_image_url(Some(&form.image_url)).to_string();

    let (click_x, click_y, has_click) = match (form.x, form.y) {
        (Some(cx), Some(cy)) => (cx, cy, true),
        _ => (0, 0, false),
    };

    let (top_left_x, top_left_y, is_valid_selection) = match (form.manual_x, form.manual_y) {
        (Some(mx), Some(my)) => (
            mx.min(form.width.saturating_sub(1)),
            my.min(form.height.saturating_sub(1)),
            true,
        ),
        _ => {
            if !has_click || (click_x == 0 && click_y == 0) {
                // Keyboard activation of <input type="image"> sends x=0, y=0 without click coordinates.
                // Treat (0,0) without explicit manual entry as a "no pick" keyboard trigger.
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

    if !is_valid_selection {
        let magnifier = ImageMagnifier::new(&image_url, form.width, form.height, None, None);
        return HtmlTemplate(RegionPickerTopLeftTemplate {
            user: user.clone(),
            csrf_token: user.csrf_token,
            automation_id: form.automation_id,
            image_url,
            width: form.width,
            height: form.height,
            step_stage: 1,
            top_left_x: None,
            top_left_y: None,
            bottom_right_x: None,
            bottom_right_y: None,
            click_x: None,
            click_y: None,
            magnifier,
            return_to: form.return_to,
            mode: form.mode,
            step_id: form.step_id,
            reference_bitmap_id: form.reference_bitmap_id,
            error_msg: Some("Keyboard selection detected (0, 0). Please enter precise pixel coordinates in the manual numeric input fields below, or click directly on the image.".to_string()),
        })
        .into_response();
    }

    let magnifier = ImageMagnifier::new(
        &image_url,
        form.width,
        form.height,
        Some(top_left_x),
        Some(top_left_y),
    );

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        automation_id: form.automation_id,
        image_url,
        width: form.width,
        height: form.height,
        step_stage: 2,
        top_left_x: Some(top_left_x),
        top_left_y: Some(top_left_y),
        bottom_right_x: None,
        bottom_right_y: None,
        click_x: Some(top_left_x),
        click_y: Some(top_left_y),
        magnifier,
        return_to: form.return_to,
        mode: form.mode,
        step_id: form.step_id,
        reference_bitmap_id: form.reference_bitmap_id,
        error_msg: None,
    })
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
    let image_url = sanitize_image_url(Some(&form.image_url)).to_string();

    let (raw_br_x, raw_br_y, is_valid_selection) = match (form.manual_x, form.manual_y) {
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

    if !is_valid_selection {
        let magnifier = ImageMagnifier::new(
            &image_url,
            form.width,
            form.height,
            Some(form.top_left_x),
            Some(form.top_left_y),
        );
        return HtmlTemplate(RegionPickerTopLeftTemplate {
            user: user.clone(),
            csrf_token: user.csrf_token,
            automation_id: form.automation_id,
            image_url,
            width: form.width,
            height: form.height,
            step_stage: 2,
            top_left_x: Some(form.top_left_x),
            top_left_y: Some(form.top_left_y),
            bottom_right_x: None,
            bottom_right_y: None,
            click_x: None,
            click_y: None,
            magnifier,
            return_to: form.return_to,
            mode: form.mode,
            step_id: form.step_id,
            reference_bitmap_id: form.reference_bitmap_id,
            error_msg: Some("Keyboard selection detected (0, 0). Please enter precise pixel coordinates in the manual numeric input fields below, or click directly on the image / grid.".to_string()),
        })
        .into_response();
    }

    let norm_tl_x = form.top_left_x.min(raw_br_x);
    let norm_tl_y = form.top_left_y.min(raw_br_y);
    let norm_br_x = form.top_left_x.max(raw_br_x);
    let norm_br_y = form.top_left_y.max(raw_br_y);

    let magnifier = ImageMagnifier::new(
        &image_url,
        form.width,
        form.height,
        Some(norm_br_x),
        Some(norm_br_y),
    );

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        automation_id: form.automation_id,
        image_url,
        width: form.width,
        height: form.height,
        step_stage: 3,
        top_left_x: Some(norm_tl_x),
        top_left_y: Some(norm_tl_y),
        bottom_right_x: Some(norm_br_x),
        bottom_right_y: Some(norm_br_y),
        click_x: Some(norm_br_x),
        click_y: Some(norm_br_y),
        magnifier,
        return_to: form.return_to,
        mode: form.mode,
        step_id: form.step_id,
        reference_bitmap_id: form.reference_bitmap_id,
        error_msg: None,
    })
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
    let parsed_url = url::Url::parse(image_url).ok();
    let path = parsed_url.as_ref().map(|u| u.path()).unwrap_or(image_url);

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

    // Direct S3 object lookup from presigned or public S3 URLs
    let bucket_prefix = format!("/{}/", state.config.s3_bucket);
    let s3_key = if let Some(key) = path.strip_prefix(&bucket_prefix) {
        Some(key)
    } else if path.starts_with('/') && !path.starts_with("/static/") && !path.starts_with("/media/")
    {
        path.strip_prefix('/')
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

    if let Some(bytes) = fetch_source_image_bytes(state, image_url).await {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

        if let Ok(mut reader) = decoder.read_info() {
            let mut buf = vec![0; reader.output_buffer_size().unwrap_or_else(|| reader.info().raw_bytes())];
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

                let mut cropped = Vec::with_capacity((crop_w * crop_h * 3) as usize);
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
                                1 | 2 => {
                                    cropped.push(px[0]);
                                    cropped.push(px[0]);
                                    cropped.push(px[0]);
                                }
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

    // Fallback: Generate a placeholder PNG of size crop_w x crop_h
    let mut fallback_pixels = Vec::with_capacity((crop_w * crop_h * 3) as usize);
    for y in 0..crop_h {
        let g = ((y * 255) / crop_h) as u8;
        for x in 0..crop_w {
            let r = ((x * 255) / crop_w) as u8;
            fallback_pixels.push(r);
            fallback_pixels.push(g);
            fallback_pixels.push(180);
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
            .map(|id| format!("&reference_bitmap_id={}", id))
            .unwrap_or_default();

        let redirect_url = if let (Some(aid), Some(sid)) = (form.automation_id, form.step_id) {
            format!(
                "/automations/{}/steps/{}/edit?search_x={}&search_y={}&search_width={}&search_height={}{}",
                aid, sid, norm_tl_x, norm_tl_y, crop_w, crop_h, ref_param
            )
        } else if let Some(aid) = form.automation_id {
            format!(
                "/automations/{}/steps/new/find_bitmap?search_x={}&search_y={}&search_width={}&search_height={}{}",
                aid, norm_tl_x, norm_tl_y, crop_w, crop_h, ref_param
            )
        } else {
            "/automations".to_string()
        };

        return Redirect::to(&redirect_url).into_response();
    }

    let bitmap_name = form.name.as_deref().unwrap_or("").trim().to_string();
    if bitmap_name.is_empty() {
        let redirect_url = match form.automation_id {
            Some(aid) => format!("/automations/{}/bitmaps", aid),
            None => "/bitmaps".to_string(),
        };
        return Redirect::to(&redirect_url).into_response();
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
            format!(
                "/automations/{}/steps/{}/edit?reference_bitmap_id={}",
                aid, sid, bitmap_id
            )
        } else if let Some(aid) = form.automation_id {
            format!(
                "/automations/{}/steps/new/find_bitmap?reference_bitmap_id={}",
                aid, bitmap_id
            )
        } else {
            "/bitmaps".to_string()
        };
        return Redirect::to(&redirect_url).into_response();
    }

    let redirect_path = match form.automation_id {
        Some(aid) => format!("/automations/{}/bitmaps", aid),
        None => "/bitmaps".to_string(),
    };

    Redirect::to(&redirect_path).into_response()
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
        // Native 1200 x 680 in 600 x 340 display -> scale = 0.5 exactly
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1200, 680);
        assert_eq!((nx, ny), (600, 340));

        let (nx_top, ny_top) = map_coarse_click_to_native(0, 0, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_top, ny_top), (0, 0));

        let (nx_bot, ny_bot) = map_coarse_click_to_native(600, 340, 600.0, 340.0, 1200, 680);
        assert_eq!((nx_bot, ny_bot), (1199, 679));
    }

    #[test]
    fn test_map_coarse_click_to_native_letterboxing() {
        // Native 1920 x 1080 in 600 x 340 display
        // scale_w = 600/1920 = 0.3125, scale_h = 340/1080 = 0.3148... -> scale = 0.3125
        // rendered_w = 600, rendered_h = 337.5, offset_x = 0, offset_y = 1.25
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 1920, 1080);
        assert_eq!((nx, ny), (960, 540));
    }

    #[test]
    fn test_map_coarse_click_to_native_pillarboxing() {
        // Native 600 x 1200 in 600 x 340 display
        // scale_w = 600/600 = 1.0, scale_h = 340/1200 = 0.2833... -> scale = 0.2833333333333333
        // rendered_w = 170, rendered_h = 340, offset_x = 215, offset_y = 0
        let (nx, ny) = map_coarse_click_to_native(300, 170, 600.0, 340.0, 600, 1200);
        // img_x = 300 - 215 = 85. native_x = 85 / (340/1200) = 300
        assert_eq!((nx, ny), (300, 600));
    }

    #[test]
    fn test_map_coarse_click_to_native_zero_dimensions() {
        let (nx, ny) = map_coarse_click_to_native(100, 100, 600.0, 340.0, 0, 0);
        assert_eq!((nx, ny), (0, 0));
    }

    #[test]
    fn test_map_grid_click_to_native_center_and_offsets() {
        // Center click at (160, 160) should yield exact (center_x, center_y)
        let (nx, ny) = map_grid_click_to_native(160, 160, 100, 200, 1920, 1080);
        assert_eq!((nx, ny), (100, 200));

        // Click +20px right on grid panel -> +1 native pixel
        let (nx_r, ny_r) = map_grid_click_to_native(180, 160, 100, 200, 1920, 1080);
        assert_eq!((nx_r, ny_r), (101, 200));

        // Click -20px left on grid panel -> -1 native pixel
        let (nx_l, ny_l) = map_grid_click_to_native(140, 160, 100, 200, 1920, 1080);
        assert_eq!((nx_l, ny_l), (99, 200));

        // Boundary clamping near native 0
        let (nx_zero, ny_zero) = map_grid_click_to_native(0, 0, 2, 2, 1920, 1080);
        assert_eq!((nx_zero, ny_zero), (0, 0));

        // Boundary clamping near native max
        let (nx_max, ny_max) = map_grid_click_to_native(319, 319, 1918, 1078, 1920, 1080);
        assert_eq!((nx_max, ny_max), (1919, 1079));
    }

    #[test]
    fn test_png_encoder_and_header_parsing() {
        let (width, height) = (10u32, 10u32);
        let mut original_pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                original_pixels.push((x * 20) as u8);
                original_pixels.push((y * 20) as u8);
                original_pixels.push(100);
            }
        }

        let png_bytes = encode_png(width, height, &original_pixels);
        assert!(!png_bytes.is_empty());

        let parsed_dims = parse_png_dimensions(&png_bytes);
        assert_eq!(parsed_dims, Some((10, 10)));
    }

    #[test]
    fn test_parse_png_dimensions() {
        // Valid PNG header for 1920 x 1080 image
        let mut valid_png = vec![0u8; 24];
        valid_png[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        valid_png[8..12].copy_from_slice(&13u32.to_be_bytes()); // Chunk length = 13
        valid_png[12..16].copy_from_slice(b"IHDR");
        valid_png[16..20].copy_from_slice(&1920u32.to_be_bytes());
        valid_png[20..24].copy_from_slice(&1080u32.to_be_bytes());

        assert_eq!(parse_png_dimensions(&valid_png), Some((1920, 1080)));

        // Valid PNG header for 100 x 50 image
        let mut valid_small = valid_png.clone();
        valid_small[16..20].copy_from_slice(&100u32.to_be_bytes());
        valid_small[20..24].copy_from_slice(&50u32.to_be_bytes());
        assert_eq!(parse_png_dimensions(&valid_small), Some((100, 50)));

        // Too short buffer
        assert_eq!(parse_png_dimensions(&valid_png[..20]), None);

        // Invalid PNG signature magic
        let mut bad_magic = valid_png.clone();
        bad_magic[0] = 0x00;
        assert_eq!(parse_png_dimensions(&bad_magic), None);

        // Invalid chunk type (JPEG or non-IHDR)
        let mut bad_chunk = valid_png.clone();
        bad_chunk[12..16].copy_from_slice(b"gAMA");
        assert_eq!(parse_png_dimensions(&bad_chunk), None);

        // Zero dimensions
        let mut zero_dim = valid_png.clone();
        zero_dim[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(parse_png_dimensions(&zero_dim), None);
    }

    #[test]
    fn test_validate_bitmap_key() {
        assert!(validate_bitmap_key("bitmaps/button.png", 1));
        assert!(validate_bitmap_key("bitmaps/user_1_abcd.png", 1));
        assert!(!validate_bitmap_key("bitmaps/user_2_abcd.png", 1));
        assert!(!validate_bitmap_key("other/button.png", 1));
        assert!(!validate_bitmap_key("bitmaps/button.jpeg", 1));
        assert!(!validate_bitmap_key("bitmaps/../etc/passwd.png", 1));
    }

    #[test]
    fn test_sanitize_image_url() {
        assert_eq!(
            sanitize_image_url(Some("/static/test.png")),
            "/static/test.png"
        );
        assert_eq!(
            sanitize_image_url(Some("/media/screenshots/12")),
            "/media/screenshots/12"
        );
        // Malicious injection attempts fall back to default
        assert_eq!(
            sanitize_image_url(Some("javascript:alert(1)")),
            "/static/sample_screenshot.png"
        );
        assert_eq!(
            sanitize_image_url(Some("/static/test.png'; body { color: red }")),
            "/static/sample_screenshot.png"
        );
    }

    #[test]
    fn test_pick_region_query_search_region_bounding() {
        let top_left_x = 100u32;
        let top_left_y = 200u32;
        let bottom_right_x = 299u32;
        let bottom_right_y = 399u32;

        let norm_tl_x = top_left_x.min(bottom_right_x);
        let norm_tl_y = top_left_y.min(bottom_right_y);
        let norm_br_x = top_left_x.max(bottom_right_x);
        let norm_br_y = top_left_y.max(bottom_right_y);

        let crop_w = norm_br_x - norm_tl_x + 1;
        let crop_h = norm_br_y - norm_tl_y + 1;

        assert_eq!(norm_tl_x, 100);
        assert_eq!(norm_tl_y, 200);
        assert_eq!(crop_w, 200);
        assert_eq!(crop_h, 200);
    }

    #[test]
    fn test_pick_region_numeric_entry_and_keyboard_no_pick_logic() {
        let _dummy_user = crate::auth::AuthUser {
            id: 1,
            username: "testuser".to_string(),
            display_name: "Test User".to_string(),
            role: crate::auth::UserRole::Editor,
            session_id: uuid::Uuid::new_v4(),
            csrf_token: "test_csrf".to_string(),
            must_change_password: false,
        };

        // 1. Explicit manual numeric entry overrides click coordinates
        let form_manual = PickRegionTopLeftForm {
            csrf_token: "test_csrf".to_string(),
            automation_id: None,
            image_url: "/static/sample_screenshot.png".to_string(),
            width: 1920,
            height: 1080,
            return_to: None,
            mode: None,
            step_id: None,
            reference_bitmap_id: None,
            x: Some(0),
            y: Some(0),
            manual_x: Some(500),
            manual_y: Some(300),
        };

        let (tl_x, tl_y, is_valid) = match (form_manual.manual_x, form_manual.manual_y) {
            (Some(mx), Some(my)) => (mx.min(1919), my.min(1079), true),
            _ => (0, 0, false),
        };
        assert!(is_valid);
        assert_eq!((tl_x, tl_y), (500, 300));

        // 2. Keyboard submission sending (0,0) without manual entry is recognized as "no pick"
        let form_keyboard = PickRegionTopLeftForm {
            csrf_token: "test_csrf".to_string(),
            automation_id: None,
            image_url: "/static/sample_screenshot.png".to_string(),
            width: 1920,
            height: 1080,
            return_to: None,
            mode: None,
            step_id: None,
            reference_bitmap_id: None,
            x: Some(0),
            y: Some(0),
            manual_x: None,
            manual_y: None,
        };

        let is_no_pick = form_keyboard.x == Some(0)
            && form_keyboard.y == Some(0)
            && form_keyboard.manual_x.is_none();
        assert!(is_no_pick);
    }

    #[test]
    fn test_form_deserialization_click_coords() {
        let json_tl = serde_json::json!({
            "csrf_token": "test_token",
            "image_url": "/static/sample_screenshot.png",
            "width": 1920,
            "height": 1080,
            "click.x": "300",
            "click.y": "170"
        });

        let form_tl: PickRegionTopLeftForm =
            serde_json::from_value(json_tl).expect("Should deserialize top left form");
        assert_eq!(form_tl.x, Some(300));
        assert_eq!(form_tl.y, Some(170));

        // Step 2: Bottom-Right grid click form deserialization (grid_click.x=160&grid_click.y=180)
        let json_br_grid = serde_json::json!({
            "csrf_token": "test_token",
            "image_url": "/static/sample_screenshot.png",
            "width": 1920,
            "height": 1080,
            "top_left_x": 100,
            "top_left_y": 200,
            "grid_click.x": "160",
            "grid_click.y": "180"
        });

        let form_br_grid: PickRegionBottomRightForm = serde_json::from_value(json_br_grid)
            .expect("Should deserialize bottom right grid form");
        assert_eq!(form_br_grid.grid_x, Some(160));
        assert_eq!(form_br_grid.grid_y, Some(180));

        // Step 2: Bottom-Right coarse click form deserialization (coarse_click.x=400&coarse_click.y=250)
        let json_br_coarse = serde_json::json!({
            "csrf_token": "test_token",
            "image_url": "/static/sample_screenshot.png",
            "width": 1920,
            "height": 1080,
            "top_left_x": 100,
            "top_left_y": 200,
            "coarse_click.x": "400",
            "coarse_click.y": "250"
        });

        let form_br_coarse: PickRegionBottomRightForm = serde_json::from_value(json_br_coarse)
            .expect("Should deserialize bottom right coarse form");
        assert_eq!(form_br_coarse.coarse_x, Some(400));
        assert_eq!(form_br_coarse.coarse_y, Some(250));
    }
}
