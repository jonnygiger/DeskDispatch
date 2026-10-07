use std::time::Duration;
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::magnifier::ImageMagnifier;
use crate::routes::auth::HtmlTemplate;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct DevMagnifierQuery {
    pub url: Option<String>,
    pub native_w: Option<u32>,
    pub native_h: Option<u32>,
    pub target_x: Option<u32>,
    pub target_y: Option<u32>,
}

#[derive(Template)]
#[template(path = "dev_magnifier.html")]
pub struct DevMagnifierTemplate {
    pub magnifier: ImageMagnifier,
}

/// GET /dev/magnifier-verify
/// Throwaway internal route to manually verify the CSS zoom and pan mathematics of the magnifier component.
#[tracing::instrument]
pub async fn dev_magnifier_verify_handler(
    Query(query): Query<DevMagnifierQuery>,
) -> impl IntoResponse {
    let url = query
        .url
        .unwrap_or_else(|| "https://via.placeholder.com/800x600.png".to_string());
    let native_w = query.native_w.unwrap_or(800);
    let native_h = query.native_h.unwrap_or(600);
    let target_x = query.target_x.or(Some(100));
    let target_y = query.target_y.or(Some(150));

    let magnifier = ImageMagnifier::new(url, native_w, native_h, target_x, target_y);

    HtmlTemplate(DevMagnifierTemplate {
        magnifier,
    })
}

/// GET /media/screenshots/{id}
/// Generates a presigned GET URL for a step screenshot and issues a 302 Found redirect with Cache-Control headers.
#[tracing::instrument(skip(state, _user))]
pub async fn get_media_screenshot_handler(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT object_storage_key FROM step_screenshots WHERE step_id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(row)) => {
            let object_key: String = row.get("object_storage_key");
            match state
                .storage_service()
                .generate_presigned_get_url(&object_key, Duration::from_secs(900))
                .await
            {
                Ok(presigned_url) => (
                    StatusCode::FOUND,
                    [
                        (header::LOCATION, presigned_url),
                        (header::CACHE_CONTROL, "private, max-age=300".to_string()),
                    ],
                )
                    .into_response(),
                Err(err) => {
                    tracing::error!("Failed to generate presigned GET URL for screenshot {}: {}", id, err);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate media URL").into_response()
                }
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Screenshot not found").into_response(),
        Err(err) => {
            tracing::error!("Database error fetching screenshot {}: {}", id, err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// GET /media/bitmaps/{id}
/// Generates a presigned GET URL for a bitmap reference image and issues a 302 Found redirect with Cache-Control headers.
#[tracing::instrument(skip(state, _user))]
pub async fn get_media_bitmap_handler(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT object_storage_key FROM bitmaps WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(row)) => {
            let object_key: String = row.get("object_storage_key");
            match state
                .storage_service()
                .generate_presigned_get_url(&object_key, Duration::from_secs(900))
                .await
            {
                Ok(presigned_url) => (
                    StatusCode::FOUND,
                    [
                        (header::LOCATION, presigned_url),
                        (header::CACHE_CONTROL, "private, max-age=300".to_string()),
                    ],
                )
                    .into_response(),
                Err(err) => {
                    tracing::error!("Failed to generate presigned GET URL for bitmap {}: {}", id, err);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate media URL").into_response()
                }
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Bitmap not found").into_response(),
        Err(err) => {
            tracing::error!("Database error fetching bitmap {}: {}", id, err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}
