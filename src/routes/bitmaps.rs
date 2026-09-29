use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use sqlx::Row;
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, AuthUser, UserRole};
use crate::magnifier::ImageMagnifier;
use crate::picker::map_coarse_click_to_native;
use crate::AppState;

#[derive(serde::Deserialize)]
pub struct BitmapUploadForm {
    pub csrf_token: String,
    pub name: String,
}

#[derive(serde::Deserialize)]
pub struct DeleteBitmapForm {
    pub csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct BitmapCommitQuery {
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
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionTopLeftForm {
    pub csrf_token: String,
    pub automation_id: Option<i64>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    #[serde(alias = "click.x", default)]
    pub x: u32,
    #[serde(alias = "click.y", default)]
    pub y: u32,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionBottomRightForm {
    pub csrf_token: String,
    pub automation_id: Option<i64>,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub top_left_x: u32,
    pub top_left_y: u32,
    #[serde(alias = "grid_click.x", default)]
    pub grid_x: Option<u32>,
    #[serde(alias = "grid_click.y", default)]
    pub grid_y: Option<u32>,
    #[serde(alias = "coarse_click.x", default)]
    pub coarse_x: Option<u32>,
    #[serde(alias = "coarse_click.y", default)]
    pub coarse_y: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct PickRegionConfirmForm {
    pub csrf_token: String,
    pub automation_id: Option<i64>,
    pub name: String,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub top_left_x: u32,
    pub top_left_y: u32,
    pub bottom_right_x: u32,
    pub bottom_right_y: u32,
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

/// GET /bitmaps
/// Renders the list page for all reference bitmaps in the system.
pub async fn get_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let rows = match sqlx::query(
        r#"
        SELECT
            b.id,
            b.automation_id,
            a.name AS automation_name,
            b.name,
            b.object_storage_key,
            b.width,
            b.height,
            b.created_by,
            COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
            b.created_at
        FROM bitmaps b
        LEFT JOIN automations a ON b.automation_id = a.id
        LEFT JOIN users u ON b.created_by = u.id
        ORDER BY b.created_at DESC, b.id DESC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching bitmaps: {}", e);
            Vec::new()
        }
    };

    let bitmaps = rows
        .into_iter()
        .map(|row| {
            let id: i64 = row.get("id");
            let width: i32 = row.get("width");
            let height: i32 = row.get("height");

            let magnifier = ImageMagnifier::new(
                format!("/media/bitmaps/{}", id),
                width.max(1) as u32,
                height.max(1) as u32,
                None,
                None,
            );

            BitmapListItem {
                id,
                automation_id: row.get("automation_id"),
                automation_name: row.get("automation_name"),
                name: row.get("name"),
                object_storage_key: row.get("object_storage_key"),
                width,
                height,
                created_by: row.get("created_by"),
                created_by_name: row.get("created_by_name"),
                created_at: row.get("created_at"),
                magnifier,
            }
        })
        .collect();

    HtmlTemplate(BitmapsListTemplate {
        user,
        csrf_token,
        bitmaps,
        automation_id: None,
        automation_name: None,
    })
}

/// GET /automations/{id}/bitmaps
/// Renders the reference bitmaps list page scoped to a specific automation.
pub async fn get_automation_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let auto_row = match sqlx::query("SELECT name FROM automations WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        _ => return Redirect::to("/automations").into_response(),
    };
    let automation_name: String = auto_row.get("name");

    let rows = match sqlx::query(
        r#"
        SELECT
            b.id,
            b.automation_id,
            a.name AS automation_name,
            b.name,
            b.object_storage_key,
            b.width,
            b.height,
            b.created_by,
            COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
            b.created_at
        FROM bitmaps b
        LEFT JOIN automations a ON b.automation_id = a.id
        LEFT JOIN users u ON b.created_by = u.id
        WHERE b.automation_id = $1 OR b.automation_id IS NULL
        ORDER BY b.created_at DESC, b.id DESC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching automation bitmaps: {}", e);
            Vec::new()
        }
    };

    let bitmaps = rows
        .into_iter()
        .map(|row| {
            let bitmap_id: i64 = row.get("id");
            let width: i32 = row.get("width");
            let height: i32 = row.get("height");

            let magnifier = ImageMagnifier::new(
                format!("/media/bitmaps/{}", bitmap_id),
                width.max(1) as u32,
                height.max(1) as u32,
                None,
                None,
            );

            BitmapListItem {
                id: bitmap_id,
                automation_id: row.get("automation_id"),
                automation_name: row.get("automation_name"),
                name: row.get("name"),
                object_storage_key: row.get("object_storage_key"),
                width,
                height,
                created_by: row.get("created_by"),
                created_by_name: row.get("created_by_name"),
                created_at: row.get("created_at"),
                magnifier,
            }
        })
        .collect();

    HtmlTemplate(BitmapsListTemplate {
        user,
        csrf_token,
        bitmaps,
        automation_id: Some(id),
        automation_name: Some(automation_name),
    })
    .into_response()
}

/// POST /bitmaps
/// Handles initial bitmap name submission, generating an S3 presigned POST policy form.
pub async fn post_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Form(form): Form<BitmapUploadForm>,
) -> Response {
    if user.role == UserRole::Viewer {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot upload bitmaps").into_response();
    }
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_name = form.name.trim().to_string();
    if bitmap_name.is_empty() {
        return Redirect::to("/bitmaps").into_response();
    }

    let object_key = format!("bitmaps/{}.png", uuid::Uuid::new_v4());

    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");
    let scheme = if host.contains("localhost") || host.contains("127.0.0.1") {
        "http"
    } else {
        "https"
    };

    let redirect_url = format!(
        "{}://{}/bitmaps/commit?key={}&name={}",
        scheme,
        host,
        object_key,
        urlencoding::encode(&bitmap_name)
    );

    let storage = state.storage_service();
    let presigned_post = match storage
        .generate_presigned_post(&object_key, Duration::from_secs(900), 10_485_760)
        .await
    {
        Ok(post) => post,
        Err(e) => {
            tracing::error!("Failed to generate presigned POST policy: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate upload policy").into_response();
        }
    };

    let mut presigned_fields: Vec<(String, String)> = presigned_post.fields.into_iter().collect();
    presigned_fields.sort_by(|a, b| a.0.cmp(&b.0));

    HtmlTemplate(BitmapsUploadTemplate {
        user,
        csrf_token: form.csrf_token,
        bitmap_name,
        object_storage_key: object_key,
        automation_id: None,
        presigned_post_url: presigned_post.url,
        presigned_fields,
        redirect_url,
    })
    .into_response()
}

/// POST /automations/{id}/bitmaps
/// Handles initial bitmap name submission scoped to an automation ID.
pub async fn post_automation_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Form(form): Form<BitmapUploadForm>,
) -> Response {
    if user.role == UserRole::Viewer {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot upload bitmaps").into_response();
    }
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_name = form.name.trim().to_string();
    if bitmap_name.is_empty() {
        return Redirect::to(&format!("/automations/{}/bitmaps", id)).into_response();
    }

    let object_key = format!("bitmaps/{}.png", uuid::Uuid::new_v4());

    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");
    let scheme = if host.contains("localhost") || host.contains("127.0.0.1") {
        "http"
    } else {
        "https"
    };

    let redirect_url = format!(
        "{}://{}/bitmaps/commit?key={}&name={}&automation_id={}",
        scheme,
        host,
        object_key,
        urlencoding::encode(&bitmap_name),
        id
    );

    let storage = state.storage_service();
    let presigned_post = match storage
        .generate_presigned_post(&object_key, Duration::from_secs(900), 10_485_760)
        .await
    {
        Ok(post) => post,
        Err(e) => {
            tracing::error!("Failed to generate presigned POST policy: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate upload policy").into_response();
        }
    };

    let mut presigned_fields: Vec<(String, String)> = presigned_post.fields.into_iter().collect();
    presigned_fields.sort_by(|a, b| a.0.cmp(&b.0));

    HtmlTemplate(BitmapsUploadTemplate {
        user,
        csrf_token: form.csrf_token,
        bitmap_name,
        object_storage_key: object_key,
        automation_id: Some(id),
        presigned_post_url: presigned_post.url,
        presigned_fields,
        redirect_url,
    })
    .into_response()
}

/// GET /bitmaps/commit
/// S3 redirect callback following direct browser upload via presigned POST policy.
pub async fn get_bitmap_commit_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<BitmapCommitQuery>,
) -> Response {
    if user.role == UserRole::Viewer {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot upload bitmaps").into_response();
    }

    let (width, height) = match state
        .s3_client
        .get_object()
        .bucket(&state.config.s3_bucket)
        .key(&query.key)
        .send()
        .await
    {
        Ok(res) => match res.body.collect().await {
            Ok(aggregated) => {
                let bytes = aggregated.into_bytes();
                let cursor = std::io::Cursor::new(&bytes);
                match image::ImageReader::new(cursor).with_guessed_format() {
                    Ok(reader) => match reader.into_dimensions() {
                        Ok((w, h)) => (w as i32, h as i32),
                        Err(_) => (100, 100),
                    },
                    Err(_) => (100, 100),
                }
            }
            Err(_) => (100, 100),
        },
        Err(e) => {
            tracing::warn!("Could not fetch uploaded object {} for dimension probing: {}", query.key, e);
            (100, 100)
        }
    };

    let row = match sqlx::query(
        r#"
        INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(query.automation_id)
    .bind(&query.name)
    .bind(&query.key)
    .bind(width)
    .bind(height)
    .bind(user.id)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to insert bitmap into database: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let bitmap_id: i64 = row.get("id");

    let details = serde_json::json!({
        "name": query.name,
        "object_storage_key": query.key,
        "automation_id": query.automation_id,
        "width": width,
        "height": height
    });

    let _ = sqlx::query(
        r#"
        INSERT INTO audit_log (user_id, action, entity_type, entity_id, details)
        VALUES ($1, 'create', 'bitmap', $2, $3)
        "#,
    )
    .bind(user.id)
    .bind(bitmap_id)
    .bind(details)
    .execute(&state.db)
    .await;

    if let Some(aid) = query.automation_id {
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
    if !user.role.can_edit() {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot delete bitmaps").into_response();
    }

    if csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_row = match sqlx::query("SELECT id, automation_id, name, object_storage_key FROM bitmaps WHERE id = $1")
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

    let auto_id: Option<i64> = bitmap_row.get("automation_id");
    let bitmap_name: String = bitmap_row.get("name");
    let object_key: String = bitmap_row.get("object_storage_key");

    if let Err(e) = state
        .s3_client
        .delete_object()
        .bucket(&state.config.s3_bucket)
        .key(&object_key)
        .send()
        .await
    {
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
pub async fn post_delete_bitmap_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<DeleteBitmapForm>,
) -> Response {
    delete_bitmap_logic(&state, &user, id, &form.csrf_token, None).await
}

/// POST /automations/{id}/bitmaps/{bid}/delete
/// Handles bitmap deletion scoped to an automation ID.
pub async fn post_automation_delete_bitmap_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, bid)): Path<(i64, i64)>,
    Form(form): Form<DeleteBitmapForm>,
) -> Response {
    delete_bitmap_logic(&state, &user, bid, &form.csrf_token, Some(id)).await
}

/// GET /bitmaps/pick-region
/// Renders initial Step 1 of two-click region picker to capture top-left corner.
pub async fn get_pick_region_handler(
    user: AuthUser,
    Query(query): Query<PickRegionQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let image_url = query.image_url.unwrap_or_else(|| "/static/sample_screenshot.png".to_string());
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
    })
}

/// GET /automations/{id}/bitmaps/pick-region
/// Renders initial Step 1 of two-click region picker scoped to an automation ID.
pub async fn get_automation_pick_region_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<PickRegionQuery>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();
    let image_url = query.image_url.unwrap_or_else(|| "/static/sample_screenshot.png".to_string());
    let width = query.width.unwrap_or(1920);
    let height = query.height.unwrap_or(1080);

    let magnifier = ImageMagnifier::new(&image_url, width, height, None, None);

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user,
        csrf_token,
        automation_id: Some(id),
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
    })
}

/// POST /bitmaps/pick-region
/// Processes coarse top-left image input click coordinates for region selection.
pub async fn post_pick_region_top_left_handler(
    user: AuthUser,
    Form(form): Form<PickRegionTopLeftForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let (top_left_x, top_left_y) = map_coarse_click_to_native(
        form.x,
        form.y,
        600.0,
        340.0,
        form.width,
        form.height,
    );

    let magnifier = ImageMagnifier::new(
        &form.image_url,
        form.width,
        form.height,
        Some(top_left_x),
        Some(top_left_y),
    );

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        automation_id: form.automation_id,
        image_url: form.image_url,
        width: form.width,
        height: form.height,
        step_stage: 2,
        top_left_x: Some(top_left_x),
        top_left_y: Some(top_left_y),
        bottom_right_x: None,
        bottom_right_y: None,
        click_x: Some(form.x),
        click_y: Some(form.y),
        magnifier,
    })
    .into_response()
}

/// POST /automations/{id}/bitmaps/pick-region
/// Processes coarse top-left image input click coordinates scoped to an automation ID.
pub async fn post_automation_pick_region_top_left_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<PickRegionTopLeftForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let (top_left_x, top_left_y) = map_coarse_click_to_native(
        form.x,
        form.y,
        600.0,
        340.0,
        form.width,
        form.height,
    );

    let magnifier = ImageMagnifier::new(
        &form.image_url,
        form.width,
        form.height,
        Some(top_left_x),
        Some(top_left_y),
    );

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        automation_id: Some(id),
        image_url: form.image_url,
        width: form.width,
        height: form.height,
        step_stage: 2,
        top_left_x: Some(top_left_x),
        top_left_y: Some(top_left_y),
        bottom_right_x: None,
        bottom_right_y: None,
        click_x: Some(form.x),
        click_y: Some(form.y),
        magnifier,
    })
    .into_response()
}

/// POST /bitmaps/pick-region/bottom-right
/// Processes grid view or coarse image click coordinates for bottom-right corner selection.
pub async fn post_pick_region_bottom_right_handler(
    user: AuthUser,
    Form(form): Form<PickRegionBottomRightForm>,
) -> Response {
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let (raw_br_x, raw_br_y) = if let (Some(gx), Some(gy)) = (form.grid_x, form.grid_y) {
        map_grid_click_to_native(
            gx,
            gy,
            form.top_left_x,
            form.top_left_y,
            form.width,
            form.height,
        )
    } else if let (Some(cx), Some(cy)) = (form.coarse_x, form.coarse_y) {
        map_coarse_click_to_native(
            cx,
            cy,
            600.0,
            340.0,
            form.width,
            form.height,
        )
    } else {
        (form.top_left_x, form.top_left_y)
    };

    let norm_tl_x = form.top_left_x.min(raw_br_x);
    let norm_tl_y = form.top_left_y.min(raw_br_y);
    let norm_br_x = form.top_left_x.max(raw_br_x);
    let norm_br_y = form.top_left_y.max(raw_br_y);

    let magnifier = ImageMagnifier::new(
        &form.image_url,
        form.width,
        form.height,
        Some(norm_br_x),
        Some(norm_br_y),
    );

    HtmlTemplate(RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: user.csrf_token,
        automation_id: form.automation_id,
        image_url: form.image_url,
        width: form.width,
        height: form.height,
        step_stage: 3,
        top_left_x: Some(norm_tl_x),
        top_left_y: Some(norm_tl_y),
        bottom_right_x: Some(norm_br_x),
        bottom_right_y: Some(norm_br_y),
        click_x: form.grid_x.or(form.coarse_x),
        click_y: form.grid_y.or(form.coarse_y),
        magnifier,
    })
    .into_response()
}

/// POST /automations/{id}/bitmaps/pick-region/bottom-right
/// Processes grid view or coarse image click coordinates for bottom-right corner selection scoped to an automation ID.
pub async fn post_automation_pick_region_bottom_right_handler(
    user: AuthUser,
    Path(id): Path<i64>,
    Form(mut form): Form<PickRegionBottomRightForm>,
) -> Response {
    form.automation_id = Some(id);
    post_pick_region_bottom_right_handler(user, Form(form)).await
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

    // Attempt 1: If image_url starts with /static/, try loading from StaticAssets
    let mut image_bytes: Option<Vec<u8>> = None;
    if let Some(static_path) = image_url.strip_prefix("/static/") {
        if let Some(asset) = crate::routes::static_assets::Assets::get(static_path) {
            image_bytes = Some(asset.data.to_vec());
        }
    }

    // Attempt 2: If image_url matches /media/screenshots/{id} or /media/bitmaps/{id}
    if image_bytes.is_none() {
        if let Some(id_str) = image_url.strip_prefix("/media/screenshots/") {
            if let Ok(id) = id_str.parse::<i64>() {
                if let Ok(Some(row)) = sqlx::query("SELECT object_storage_key FROM step_screenshots WHERE step_id = $1")
                    .bind(id)
                    .fetch_optional(&state.db)
                    .await
                {
                    let key: String = row.get("object_storage_key");
                    if let Ok(res) = state.s3_client.get_object().bucket(&state.config.s3_bucket).key(&key).send().await {
                        if let Ok(data) = res.body.collect().await {
                            image_bytes = Some(data.into_bytes().to_vec());
                        }
                    }
                }
            }
        } else if let Some(id_str) = image_url.strip_prefix("/media/bitmaps/") {
            if let Ok(id) = id_str.parse::<i64>() {
                if let Ok(Some(row)) = sqlx::query("SELECT object_storage_key FROM bitmaps WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&state.db)
                    .await
                {
                    let key: String = row.get("object_storage_key");
                    if let Ok(res) = state.s3_client.get_object().bucket(&state.config.s3_bucket).key(&key).send().await {
                        if let Ok(data) = res.body.collect().await {
                            image_bytes = Some(data.into_bytes().to_vec());
                        }
                    }
                }
            }
        }
    }

    // Try decoding and cropping loaded image_bytes
    if let Some(bytes) = image_bytes {
        if let Ok(img) = image::load_from_memory(&bytes) {
            let cropped = img.crop_imm(tl_x, tl_y, crop_w, crop_h);
            let mut buf = std::io::Cursor::new(Vec::new());
            if cropped.write_to(&mut buf, image::ImageFormat::Png).is_ok() {
                return buf.into_inner();
            }
        }
    }

    // Fallback: Generate a placeholder PNG of size crop_w x crop_h
    let img_buf = image::RgbImage::from_fn(crop_w, crop_h, |x, y| {
        let r = ((x * 255) / crop_w.max(1)) as u8;
        let g = ((y * 255) / crop_h.max(1)) as u8;
        image::Rgb([r, g, 180])
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = image::DynamicImage::ImageRgb8(img_buf).write_to(&mut buf, image::ImageFormat::Png);
    buf.into_inner()
}

pub async fn confirm_region_crop_logic(
    state: &AppState,
    user: &AuthUser,
    form: PickRegionConfirmForm,
) -> Response {
    if !user.role.can_edit() {
        return (StatusCode::FORBIDDEN, "Forbidden: Viewers cannot create bitmaps").into_response();
    }
    if form.csrf_token != user.csrf_token {
        return (StatusCode::BAD_REQUEST, "Invalid CSRF token").into_response();
    }

    let bitmap_name = form.name.trim().to_string();
    if bitmap_name.is_empty() {
        let redirect_path = match form.automation_id {
            Some(aid) => format!("/automations/{}/bitmaps", aid),
            None => "/bitmaps".to_string(),
        };
        return Redirect::to(&redirect_path).into_response();
    }

    let norm_tl_x = form.top_left_x.min(form.bottom_right_x);
    let norm_tl_y = form.top_left_y.min(form.bottom_right_y);
    let norm_br_x = form.top_left_x.max(form.bottom_right_x);
    let norm_br_y = form.top_left_y.max(form.bottom_right_y);

    let crop_w = norm_br_x - norm_tl_x + 1;
    let crop_h = norm_br_y - norm_tl_y + 1;

    let object_key = format!("bitmaps/crop_{}.png", uuid::Uuid::new_v4());

    let png_bytes = crop_image_region(state, &form.image_url, norm_tl_x, norm_tl_y, crop_w, crop_h).await;

    if let Err(e) = state
        .s3_client
        .put_object()
        .bucket(&state.config.s3_bucket)
        .key(&object_key)
        .content_type("image/png")
        .body(png_bytes.into())
        .send()
        .await
    {
        tracing::error!("Failed to upload cropped bitmap to S3: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store bitmap image").into_response();
    }

    let row = match sqlx::query(
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
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to insert cropped bitmap into database: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let bitmap_id: i64 = row.get("id");

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

    let redirect_path = match form.automation_id {
        Some(aid) => format!("/automations/{}/bitmaps", aid),
        None => "/bitmaps".to_string(),
    };

    Redirect::to(&redirect_path).into_response()
}

/// POST /bitmaps/pick-region/confirm
/// Confirms and saves a selected region crop as a reference bitmap.
pub async fn post_pick_region_confirm_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<PickRegionConfirmForm>,
) -> Response {
    confirm_region_crop_logic(&state, &user, form).await
}

/// POST /automations/{id}/bitmaps/pick-region/confirm
/// Confirms and saves a selected region crop as a reference bitmap scoped to an automation ID.
pub async fn post_automation_pick_region_confirm_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(mut form): Form<PickRegionConfirmForm>,
) -> Response {
    form.automation_id = Some(id);
    confirm_region_crop_logic(&state, &user, form).await
}


/// Maps click coordinates on the 20x grid panel view (320x320 viewport centered at `center_x`, `center_y`)
/// to exact native image pixel coordinates.
pub fn map_grid_click_to_native(
    click_x: u32,
    click_y: u32,
    center_x: u32,
    center_y: u32,
    native_w: u32,
    native_h: u32,
) -> (u32, u32) {
    if native_w == 0 || native_h == 0 {
        return (0, 0);
    }

    let dx = (click_x as f64 - 160.0 + 10.0) / 20.0;
    let dy = (click_y as f64 - 160.0 + 10.0) / 20.0;

    let offset_x = dx.floor() as i64;
    let offset_y = dy.floor() as i64;

    let target_x = (center_x as i64 + offset_x).clamp(0, (native_w - 1) as i64) as u32;
    let target_y = (center_y as i64 + offset_y).clamp(0, (native_h - 1) as i64) as u32;

    (target_x, target_y)
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
}
