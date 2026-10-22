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
use deskdispatch::config::Config;
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
    let mut hasher = Sha256::new();
    hasher.update(raw_api_key.as_bytes());
    let api_key_hash: Vec<u8> = hasher.finalize().to_vec();

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

    let config = Config::from_env().unwrap();
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
