#![allow(clippy::unwrap_used)]
use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
    response::IntoResponse,
    routing::get,
};
use deskdispatch::AppState;
use deskdispatch::auth::WorkerAuth;
use deskdispatch::config::{Config, SecretString};

fn create_test_config() -> Config {
    Config {
        database_url: "postgres://postgres:postgrespassword@localhost:5432/deskdispatch"
            .to_string(),
        public_base_url: "http://localhost:3000".to_string(),
        s3_endpoint: None,
        s3_public_endpoint: None,
        s3_bucket: "deskdispatch-bucket".to_string(),
        s3_access_key: "test_key".to_string(),
        s3_secret_key: SecretString::from("test_secret"),
        s3_region: "us-east-1".to_string(),
        session_secret: SecretString::from("test_session_secret"),
        bind_address: "0.0.0.0:3000".to_string(),
        environment: "development".to_string(),
        min_agent_version: None,
        worker_poll_interval_secs: 5,
        worker_heartbeat_interval_secs: 15,
        trust_proxy_headers: false,
        database_max_connections: 5,
        database_acquire_timeout_secs: 5,
        worker_long_poll_timeout_secs: 10,
        session_retention_days: 7,
        audit_log_retention_days: 90,
        task_run_step_retention_days: 30,
        screenshot_retention_days: 30,
        metrics_bind_address: None,
        unattended_registration_psk: SecretString::from("test_psk"),
    }
}
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

async fn test_protected_worker_route(worker: WorkerAuth) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "authenticated",
            "worker_id": worker.id,
            "hostname": worker.hostname,
            "display_name": worker.display_name,
        })),
    )
}

struct WorkerAuthCleanupGuard {
    pool: sqlx::PgPool,
    worker_id: i64,
}

impl Drop for WorkerAuthCleanupGuard {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let worker_id = self.worker_id;

        let cleanup = async move {
            let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
                .bind(worker_id)
                .execute(&pool)
                .await;
        };

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(cleanup);
        })
        .join()
        .unwrap();
    }
}

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect(&db_url)
        .await
        .ok()
}

#[tokio::test]
async fn test_worker_auth_extractor_integration() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping worker auth integration test");
        return;
    };

    let raw_api_key = format!("test_key_{}", Uuid::new_v4().simple());
    let api_key_hash = Sha256::digest(raw_api_key.as_bytes()).to_vec();

    let hostname = format!("worker-host-{}", Uuid::new_v4().simple());
    let display_name = "Auth Test Worker PC";

    let worker_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, api_key_hash, status)
        VALUES ($1, $2, $3, 'online')
        RETURNING id
        "#,
    )
    .bind(&hostname)
    .bind(display_name)
    .bind(&api_key_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let _cleanup_guard = WorkerAuthCleanupGuard {
        pool: pool.clone(),
        worker_id,
    };

    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        None::<String>,
    );

    let state = AppState {
        db: pool.clone(),
        storage,
        config: config.clone(),
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tokio::sync::broadcast::channel::<()>(100).0,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = Router::new()
        .route(
            "/api/v1/workers/test-auth",
            get(test_protected_worker_route),
        )
        .with_state(state);

    // 1. Missing Authorization header -> 401 Unauthorized
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 2. Malformed Authorization header -> 401 Unauthorized
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, "Basic invalidtoken123")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 3. Invalid API key -> 401 Unauthorized
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(
            AUTHORIZATION,
            "Bearer invalid_api_key_000000000000000000000",
        )
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 4. Empty Bearer API key -> 401 Unauthorized
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, "Bearer ")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 5. Valid Bearer API key -> 200 OK
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, format!("Bearer {}", raw_api_key))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(resp_json["status"], "authenticated");
    assert_eq!(resp_json["worker_id"], worker_id);
    assert_eq!(resp_json["hostname"], hostname);
    assert_eq!(resp_json["display_name"], display_name);
}

#[tokio::test]
async fn test_worker_auth_key_rotation_and_expiration() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping worker auth key rotation test");
        return;
    };

    let current_api_key = format!("curr_key_{}", Uuid::new_v4().simple());
    let current_key_hash = Sha256::digest(current_api_key.as_bytes()).to_vec();

    let previous_api_key = format!("prev_key_{}", Uuid::new_v4().simple());
    let previous_key_hash = Sha256::digest(previous_api_key.as_bytes()).to_vec();

    let hostname = format!("rotation-host-{}", Uuid::new_v4().simple());
    let display_name = "Key Rotation Test Worker PC";

    // Insert worker with active previous API key valid for 1 hour in the future
    let worker_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (
            hostname, display_name, api_key_hash, previous_api_key_hash,
            previous_api_key_expires_at, status
        )
        VALUES ($1, $2, $3, $4, now() + interval '1 hour', 'online')
        RETURNING id
        "#,
    )
    .bind(&hostname)
    .bind(display_name)
    .bind(&current_key_hash)
    .bind(&previous_key_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let _cleanup_guard = WorkerAuthCleanupGuard {
        pool: pool.clone(),
        worker_id,
    };

    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        None::<String>,
    );

    let state = AppState {
        db: pool.clone(),
        storage,
        config: config.clone(),
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tokio::sync::broadcast::channel::<()>(100).0,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = Router::new()
        .route(
            "/api/v1/workers/test-auth",
            get(test_protected_worker_route),
        )
        .with_state(state);

    // 1. Authenticate using non-expired previous API key -> 200 OK
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, format!("Bearer {}", previous_api_key))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 2. Authenticate using current API key -> 200 OK
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, format!("Bearer {}", current_api_key))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Expire the previous API key by updating previous_api_key_expires_at to 1 hour in the past
    sqlx::query(
        r#"
        UPDATE task_worker_pcs
        SET previous_api_key_expires_at = now() - interval '1 hour'
        WHERE id = $1
        "#,
    )
    .bind(worker_id)
    .execute(&pool)
    .await
    .unwrap();

    // 4. Authenticate using expired previous API key -> 401 Unauthorized
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, format!("Bearer {}", previous_api_key))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 5. Current API key still authenticates successfully -> 200 OK
    let req = Request::builder()
        .uri("/api/v1/workers/test-auth")
        .header(AUTHORIZATION, format!("Bearer {}", current_api_key))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
