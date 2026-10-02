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

use crate::{auth::AuthWorker, AppState};

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

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub status: Option<String>,
    pub current_task_run_id: Option<i64>,
    pub screen_width: Option<i32>,
    pub screen_height: Option<i32>,
    pub os_info: Option<String>,
    pub agent_version: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HeartbeatResponse {
    pub status: String,
    pub cancel_requested: bool,
}

pub fn is_valid_worker_status(status: &str) -> bool {
    matches!(status, "offline" | "online" | "busy" | "error")
}

pub async fn post_heartbeat_handler(
    worker: AuthWorker,
    State(state): State<AppState>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    let new_status = if let Some(ref status_str) = payload.status {
        let trimmed = status_str.trim();
        if !is_valid_worker_status(trimmed) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid status '{}'. Expected one of: offline, online, busy, error", status_str),
                }),
            )
                .into_response();
        }
        trimmed.to_string()
    } else {
        // Default to 'online' if worker is sending a heartbeat without explicit status change, or keep current status if busy?
        // Spec: "Body: {status, current_task_run_id?}"
        // If status is omitted, we keep worker's existing status unless it was offline/error where online is default, or just fallback to current status/online.
        if is_valid_worker_status(&worker.status) {
            worker.status.clone()
        } else {
            "online".to_string()
        }
    };

    let update_result = sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET
            last_heartbeat_at = now(),
            status = $1,
            screen_width = COALESCE($2, screen_width),
            screen_height = COALESCE($3, screen_height),
            os_info = COALESCE($4, os_info),
            agent_version = COALESCE($5, agent_version)
        WHERE id = $6
        "#,
    )
    .bind(&new_status)
    .bind(payload.screen_width)
    .bind(payload.screen_height)
    .bind(payload.os_info.as_deref())
    .bind(payload.agent_version.as_deref())
    .bind(worker.id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_result {
        tracing::error!(worker_id = worker.id, "Failed to update heartbeat: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Database error during heartbeat".to_string(),
            }),
        )
            .into_response();
    }

    let mut cancel_requested = false;

    if let Some(task_run_id) = payload.current_task_run_id {
        let run_status_res: Result<Option<String>, _> = sqlx::query_scalar(
            r#"
            SELECT status FROM task_runs WHERE id = $1 AND worker_id = $2
            "#,
        )
        .bind(task_run_id)
        .bind(worker.id)
        .fetch_optional(&state.db)
        .await;

        match run_status_res {
            Ok(Some(status)) => {
                if status == "cancelled" {
                    cancel_requested = true;
                }
            }
            Ok(None) => {
                // If run doesn't exist or isn't assigned to this worker, flag cancellation
                cancel_requested = true;
            }
            Err(e) => {
                tracing::error!(
                    worker_id = worker.id,
                    task_run_id = task_run_id,
                    "Error querying task run status for heartbeat cancel flag: {}",
                    e
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(HeartbeatResponse {
            status: "success".to_string(),
            cancel_requested,
        }),
    )
        .into_response()
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

    #[test]
    fn test_is_valid_worker_status() {
        assert!(is_valid_worker_status("offline"));
        assert!(is_valid_worker_status("online"));
        assert!(is_valid_worker_status("busy"));
        assert!(is_valid_worker_status("error"));

        assert!(!is_valid_worker_status("invalid"));
        assert!(!is_valid_worker_status(""));
        assert!(!is_valid_worker_status("UNKNOWN"));
    }

    #[test]
    fn test_heartbeat_request_deserialization() {
        let json_data = r#"{
            "status": "online",
            "current_task_run_id": 123,
            "screen_width": 1920,
            "screen_height": 1080,
            "os_info": "Linux x86_64",
            "agent_version": "1.0.0"
        }"#;

        let req: HeartbeatRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize HeartbeatRequest");
        assert_eq!(req.status.as_deref(), Some("online"));
        assert_eq!(req.current_task_run_id, Some(123));
        assert_eq!(req.screen_width, Some(1920));
        assert_eq!(req.screen_height, Some(1080));
        assert_eq!(req.os_info.as_deref(), Some("Linux x86_64"));
        assert_eq!(req.agent_version.as_deref(), Some("1.0.0"));

        let empty_json = r#"{}"#;
        let empty_req: HeartbeatRequest =
            serde_json::from_str(empty_json).expect("Failed to deserialize empty HeartbeatRequest");
        assert_eq!(empty_req.status, None);
        assert_eq!(empty_req.current_task_run_id, None);
    }

    #[test]
    fn test_heartbeat_response_serialization() {
        let resp = HeartbeatResponse {
            status: "success".to_string(),
            cancel_requested: true,
        };

        let json_str = serde_json::to_string(&resp).expect("Failed to serialize HeartbeatResponse");
        assert!(json_str.contains(r#""status":"success""#));
        assert!(json_str.contains(r#""cancel_requested":true"#));
    }
}
