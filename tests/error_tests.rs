#![allow(clippy::unwrap_used)]

use askama::Template;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use deskdispatch::AppState;
use deskdispatch::auth::{AuthUser, LoginRateLimiter, UserRole};
use deskdispatch::config::{Config, SecretString};
use deskdispatch::routes::{
    InternalServerErrorTemplate, NotFoundTemplate, get_index_handler, get_login_handler,
    not_found_handler,
};
use tower::ServiceExt;
use uuid::Uuid;

fn create_test_config() -> Config {
    Config {
        database_url: "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string(),
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

#[test]
fn test_not_found_template_rendering() {
    let user = AuthUser {
        id: 1,
        username: "testuser".to_string(),
        display_name: "Test User".to_string(),
        role: UserRole::Viewer,
        session_id: Uuid::new_v4(),
        csrf_token: "csrf_12345".to_string(),
        must_change_password: false,
    };

    // 1. Without user
    let tmpl_no_user = NotFoundTemplate { user: None };
    let html_no_user = tmpl_no_user.render().unwrap();
    assert!(html_no_user.contains("404"));
    assert!(html_no_user.contains("Page Not Found"));
    assert!(html_no_user.contains("Return to Dashboard"));

    // 2. With user
    let tmpl_user = NotFoundTemplate { user: Some(user) };
    let html_user = tmpl_user.render().unwrap();
    assert!(html_user.contains("404"));
    assert!(html_user.contains("Page Not Found"));
    assert!(html_user.contains("Test User"));
}

#[test]
fn test_internal_server_error_template_rendering() {
    let user = AuthUser {
        id: 2,
        username: "admin".to_string(),
        display_name: "Admin User".to_string(),
        role: UserRole::Admin,
        session_id: Uuid::new_v4(),
        csrf_token: "csrf_67890".to_string(),
        must_change_password: false,
    };

    // 1. Default message
    let tmpl_default = InternalServerErrorTemplate {
        message: None,
        user: None,
    };
    let html_default = tmpl_default.render().unwrap();
    assert!(html_default.contains("500"));
    assert!(html_default.contains("Internal Server Error"));
    assert!(html_default.contains("An unexpected error occurred on the server"));

    // 2. Custom message and user
    let tmpl_custom = InternalServerErrorTemplate {
        message: Some("Database query timed out".to_string()),
        user: Some(user),
    };
    let html_custom = tmpl_custom.render().unwrap();
    assert!(html_custom.contains("500"));
    assert!(html_custom.contains("Database query timed out"));
    assert!(html_custom.contains("Admin User"));
}

#[tokio::test]
async fn test_fallback_404_route() {
    let pool = sqlx::PgPool::connect_lazy("postgres://postgres:postgrespassword@localhost:5432/deskdispatch")
        .expect("create lazy pool");

    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        None::<String>,
    );

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool,
        storage,
        config,
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = Router::new()
        .route("/login", get(get_login_handler))
        .route("/", get(get_index_handler))
        .fallback(not_found_handler)
        .with_state(state);

    let req = Request::builder()
        .uri("/this-route-does-not-exist")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body_bytes);
    assert!(html.contains("404"));
    assert!(html.contains("Page Not Found"));
}
