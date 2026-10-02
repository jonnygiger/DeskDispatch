use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterWorkerRequest {
    pub registration_token: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterWorkerResponse {
    pub status: String,
    pub worker_id: i64,
    pub hostname: String,
    pub display_name: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn post_register_worker_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterWorkerRequest>,
) -> impl IntoResponse {
    let token = payload
        .registration_token
        .or(payload.token)
        .map(|t| t.trim().to_string())
        .unwrap_or_default();

    if token.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "registration_token is required".to_string(),
            }),
        )
            .into_response();
    }

    // Generate a 256-bit (64 hex characters) random API key
    let api_key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());

    // Compute SHA-256 hash of the bearer API key
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let api_key_hash: Vec<u8> = hasher.finalize().to_vec();

    let row_result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET api_key_hash = $1, registration_token = NULL
        WHERE registration_token = $2 AND registration_token IS NOT NULL
        RETURNING id, hostname, display_name
        "#,
    )
    .bind(&api_key_hash)
    .bind(&token)
    .fetch_optional(&state.db)
    .await;

    match row_result {
        Ok(Some(row)) => {
            let worker_id: i64 = row.get("id");
            let hostname: String = row.get("hostname");
            let display_name: String = row.get("display_name");

            tracing::info!(
                worker_id = worker_id,
                hostname = %hostname,
                "Task worker PC registered successfully"
            );

            (
                StatusCode::OK,
                Json(RegisterWorkerResponse {
                    status: "success".to_string(),
                    worker_id,
                    hostname,
                    display_name,
                    api_key,
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid or expired registration token".to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to register worker PC: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Database error during registration".to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_worker_request_deserialization() {
        let json_data = r#"{"registration_token": "token-123"}"#;
        let req: RegisterWorkerRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize registration_token");
        assert_eq!(req.registration_token.as_deref(), Some("token-123"));
        assert_eq!(req.token, None);

        let json_data_alt = r#"{"token": "token-456"}"#;
        let req_alt: RegisterWorkerRequest =
            serde_json::from_str(json_data_alt).expect("Failed to deserialize token");
        assert_eq!(req_alt.registration_token, None);
        assert_eq!(req_alt.token.as_deref(), Some("token-456"));
    }

    #[test]
    fn test_api_key_generation_and_sha256_hash() {
        let api_key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        assert_eq!(api_key.len(), 64);

        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        let hash: Vec<u8> = hasher.finalize().to_vec();
        assert_eq!(hash.len(), 32);
    }
}
