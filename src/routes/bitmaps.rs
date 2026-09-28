use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use sqlx::Row;
use std::time::Duration;

use super::auth::HtmlTemplate;
use crate::auth::{AuthUser, UserRole};
use crate::magnifier::ImageMagnifier;
use crate::AppState;

#[derive(serde::Deserialize)]
pub struct BitmapUploadForm {
    pub csrf_token: String,
    pub name: String,
}

#[derive(serde::Deserialize)]
pub struct BitmapCommitQuery {
    pub key: String,
    pub name: String,
    pub automation_id: Option<i64>,
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
