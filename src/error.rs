use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use deskdispatch_protocol::ErrorResponse;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Upgrade required: {0}")]
    UpgradeRequired(String),

    #[error("Internal server error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            }
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::UpgradeRequired(msg) => (StatusCode::UPGRADE_REQUIRED, msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Internal server error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
            }
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn test_app_error_into_response_status_and_json_payload() {
        let cases = vec![
            (
                AppError::BadRequest("invalid query parameter".to_string()),
                StatusCode::BAD_REQUEST,
                "invalid query parameter",
            ),
            (
                AppError::NotFound("worker not found".to_string()),
                StatusCode::NOT_FOUND,
                "worker not found",
            ),
            (
                AppError::Unauthorized("invalid api key".to_string()),
                StatusCode::UNAUTHORIZED,
                "invalid api key",
            ),
            (
                AppError::Forbidden("admin privileges required".to_string()),
                StatusCode::FORBIDDEN,
                "admin privileges required",
            ),
            (
                AppError::Conflict("worker hostname exists".to_string()),
                StatusCode::CONFLICT,
                "worker hostname exists",
            ),
            (
                AppError::UpgradeRequired("worker agent version outdated".to_string()),
                StatusCode::UPGRADE_REQUIRED,
                "worker agent version outdated",
            ),
            (
                AppError::Internal("failed to process job".to_string()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to process job",
            ),
            (
                AppError::Database(sqlx::Error::RowNotFound),
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            ),
        ];

        for (err, expected_status, expected_msg) in cases {
            let response = err.into_response();
            assert_eq!(response.status(), expected_status);

            let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let json_body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

            assert_eq!(json_body["error"], expected_msg);
        }
    }
}
