#![allow(clippy::unwrap_used)]

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header},
};
use deskdispatch::AppState;
use deskdispatch::auth::LoginRateLimiter;
use deskdispatch::config::{Config, SecretString};
use deskdispatch::routes::static_assets::router as static_assets_router;
use tower::ServiceExt;

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

fn create_test_state() -> AppState {
    let pool = sqlx::PgPool::connect_lazy(
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch",
    )
    .expect("create lazy pool");
    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket);
    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    AppState {
        db: pool,
        storage,
        config,
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

#[tokio::test]
async fn test_static_asset_style_css() {
    let app = static_assets_router().with_state(create_test_state());

    let req = Request::builder()
        .uri("/static/style.css")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("Content-Type header present")
        .to_str()
        .unwrap();

    assert!(
        content_type.contains("text/css"),
        "Expected text/css, got {content_type}"
    );

    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .expect("Cache-Control header present")
        .to_str()
        .unwrap();

    assert!(
        cache_control.contains("immutable"),
        "Expected immutable cache control, got {cache_control}"
    );

    assert!(
        response.headers().contains_key(header::ETAG),
        "Expected ETag header present"
    );

    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("Read response body");

    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains(":root"), "CSS content expected");
}

#[tokio::test]
async fn test_static_asset_not_found() {
    let app = static_assets_router().with_state(create_test_state());

    let req = Request::builder()
        .uri("/static/nonexistent.css")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_asset_etag_304() {
    let app = static_assets_router().with_state(create_test_state());

    let req = Request::builder()
        .uri("/static/style.css")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header present")
        .to_str()
        .unwrap()
        .to_string();

    // Exact matching ETag -> 304 NOT_MODIFIED
    let req_304 = Request::builder()
        .uri("/static/style.css")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();

    let not_modified_res = app.clone().oneshot(req_304).await.unwrap();
    assert_eq!(not_modified_res.status(), StatusCode::NOT_MODIFIED);

    // List of ETags containing valid ETag -> 304 NOT_MODIFIED
    let list_etag = format!("\"other-etag-999\", {etag}");
    let req_list_304 = Request::builder()
        .uri("/static/style.css")
        .header(header::IF_NONE_MATCH, &list_etag)
        .body(Body::empty())
        .unwrap();

    let list_res = app.clone().oneshot(req_list_304).await.unwrap();
    assert_eq!(list_res.status(), StatusCode::NOT_MODIFIED);

    // Mismatched ETag -> 200 OK
    let req_200 = Request::builder()
        .uri("/static/style.css")
        .header(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"outdated-etag-12345\""),
        )
        .body(Body::empty())
        .unwrap();

    let ok_res = app.oneshot(req_200).await.unwrap();
    assert_eq!(ok_res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_static_asset_leading_slash_path_resolution() {
    let app = static_assets_router().with_state(create_test_state());

    let req = Request::builder()
        .uri("/static//style.css")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
