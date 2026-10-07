use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;

pub fn generate_worker_api_key() -> String {
    format!("dd_pk_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn generate_registration_token() -> String {
    format!("dd_reg_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash_token(token: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().to_vec()
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AuthWorker {
    pub id: i64,
    pub hostname: String,
    pub display_name: String,
    pub status: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub type WorkerAuth = AuthWorker;

impl FromRequestParts<AppState> for AuthWorker {
    type Rejection = Response;

    #[tracing::instrument(skip(parts, state))]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok());

        let api_key = match auth_header {
            Some(header_val) => {
                let trimmed = header_val.trim();
                if trimmed.to_lowercase().starts_with("bearer ") {
                    trimmed[7..].trim()
                } else {
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Invalid Authorization header format. Expected 'Bearer <api_key>'"
                        })),
                    )
                        .into_response());
                }
            }
            None => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Missing Authorization header"
                    })),
                )
                    .into_response());
            }
        };

        if api_key.is_empty() {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "API key cannot be empty"
                })),
            )
                .into_response());
        }

        // Compute SHA-256 hash of the bearer API key
        let api_key_hash = hash_token(api_key);

        // Perform SHA-256 hash lookup in task_worker_pcs table (supporting current key or non-expired previous key during rotation)
        let worker = sqlx::query_as::<_, AuthWorker>(
            r#"
            SELECT
                id, hostname, display_name, status, last_heartbeat_at,
                screen_width, screen_height, os_info, agent_version, created_at
            FROM task_worker_pcs
            WHERE api_key_hash = $1
               OR (previous_api_key_hash = $1 AND previous_api_key_expires_at > now())
            "#,
        )
        .bind(&api_key_hash)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database query error in AuthWorker extractor: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Database error during worker authentication"
                })),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Invalid worker API key"
                })),
            )
                .into_response()
        })?;

        Ok(worker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_auth_sha256_hashing() {
        let api_key = "test_api_key_1234567890_sample";
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        let hash: Vec<u8> = hasher.finalize().to_vec();

        assert_eq!(hash.len(), 32);

        // Verify that same key produces exact same hash
        let mut hasher2 = Sha256::new();
        hasher2.update(api_key.as_bytes());
        assert_eq!(hash, hasher2.finalize().to_vec());
    }
}
