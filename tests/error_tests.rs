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
use deskdispatch::config::Config;
use deskdispatch::routes::{
    InternalServerErrorTemplate, NotFoundTemplate, get_index_handler, get_login_handler,
    not_found_handler,
};
use tower::ServiceExt;
use uuid::Uuid;

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

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::PgPool::connect(&db_url).await.ok()
}

#[tokio::test]
async fn test_fallback_404_route() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = Config::from_env().unwrap();
    let credentials = aws_sdk_s3::config::Credentials::new("key", "secret", None, None, "static");
    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .build();
    let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool,
        s3_client,
        config,
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
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
