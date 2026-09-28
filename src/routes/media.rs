use std::time::Duration;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use sqlx::Row;

use crate::auth::AuthUser;
use crate::AppState;

/// GET /media/screenshots/{id}
/// Generates a presigned GET URL for a step screenshot and issues a 302 Found redirect.
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
                    [(header::LOCATION, presigned_url)],
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
/// Generates a presigned GET URL for a bitmap reference image and issues a 302 Found redirect.
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
                    [(header::LOCATION, presigned_url)],
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
