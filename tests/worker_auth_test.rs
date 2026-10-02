use app::auth::WorkerAuth;
use app::config::Config;
use app::AppState;
use axum::{
    body::Body,
    http::{header::AUTHORIZATION, Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

async fn test_protected_worker_route(
    worker: WorkerAuth,
) -> impl IntoResponse {
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

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::PgPool::connect(&db_url).await.ok()
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
    .bind(&display_name)
    .bind(&api_key_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let config = Config::from_env().unwrap();
    let credentials = aws_sdk_s3::config::Credentials::new("key", "secret", None, None, "static");
    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .build();
    let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

    let state = AppState {
        db: pool.clone(),
        s3_client,
        config: config.clone(),
        rate_limiter: app::auth::LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/api/v1/workers/test-auth", get(test_protected_worker_route))
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
        .header(AUTHORIZATION, "Bearer invalid_api_key_000000000000000000000")
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

    // Clean up test worker
    let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
        .bind(worker_id)
        .execute(&pool)
        .await;
}
