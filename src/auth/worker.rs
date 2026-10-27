use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;

pub fn generate_worker_api_key() -> String {
    format!(
        "dd_pk_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

pub fn generate_registration_token() -> String {
    format!(
        "dd_reg_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
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
        let hash = hash_token(api_key);

        assert_eq!(hash.len(), 32);

        // Verify that same key produces exact same hash via hash_token helper
        let hash2 = hash_token(api_key);
        assert_eq!(hash, hash2);

        // Different keys produce different hashes
        let diff_hash = hash_token("different_api_key");
        assert_ne!(hash, diff_hash);
    }

    #[test]
    fn test_token_generation_format_and_uniqueness() {
        let key1 = generate_worker_api_key();
        let key2 = generate_worker_api_key();

        assert!(key1.starts_with("dd_pk_"));
        assert!(key2.starts_with("dd_pk_"));
        assert_ne!(key1, key2);
        // dd_pk_ (6 chars) + 2x 32-char simple UUIDs = 70 chars
        assert_eq!(key1.len(), 70);

        let reg1 = generate_registration_token();
        let reg2 = generate_registration_token();

        assert!(reg1.starts_with("dd_reg_"));
        assert!(reg2.starts_with("dd_reg_"));
        assert_ne!(reg1, reg2);
        // dd_reg_ (7 chars) + 2x 32-char simple UUIDs = 71 chars
        assert_eq!(reg1.len(), 71);
    }
}
