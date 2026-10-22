#![allow(clippy::unwrap_used)]

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use deskdispatch::AppState;
use deskdispatch::auth::LoginRateLimiter;
use deskdispatch::config::Config;
use deskdispatch::routes::static_assets::router as static_assets_router;
use tower::ServiceExt;

fn create_test_state() -> AppState {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    let pool = sqlx::PgPool::connect_lazy(&db_url).expect("create lazy pool");
    let config = Config::from_env().unwrap();
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

    let req_304 = Request::builder()
        .uri("/static/style.css")
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .unwrap();

    let not_modified_res = app.oneshot(req_304).await.unwrap();
    assert_eq!(not_modified_res.status(), StatusCode::NOT_MODIFIED);
}
