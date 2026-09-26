use app::auth::{
    clear_session_cookie, create_session_cookie, generate_csrf_token,
    validate_csrf_token, LoginRateLimiter, UserRole,
};
use app::config::Config;
use app::routes::{
    get_index_handler, get_login_handler, get_password_handler, post_login_handler,
    post_logout_handler, post_password_handler,
};
use app::AppState;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use sqlx::Row;
use std::net::IpAddr;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn test_argon2_password_hashing() {
    let password = "SuperSecretPassword123!";
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();

    let parsed_hash = PasswordHash::new(&password_hash).unwrap();
    assert!(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok());

    assert!(argon2
        .verify_password("WrongPassword".as_bytes(), &parsed_hash)
        .is_err());
}

#[test]
fn test_user_role_permissions() {
    let admin = UserRole::Admin;
    let editor = UserRole::Editor;
    let viewer = UserRole::Viewer;

    assert!(admin.is_admin());
    assert!(admin.can_edit());

    assert!(!editor.is_admin());
    assert!(editor.can_edit());

    assert!(!viewer.is_admin());
    assert!(!viewer.can_edit());

    assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
    assert_eq!("editor".parse::<UserRole>().unwrap(), UserRole::Editor);
    assert_eq!("viewer".parse::<UserRole>().unwrap(), UserRole::Viewer);
    assert!("invalid".parse::<UserRole>().is_err());
}

#[test]
fn test_csrf_token_generation_and_validation() {
    let session_id = Uuid::new_v4();
    let secret = "test_secret_key_12345";

    let token = generate_csrf_token(session_id, secret);
    assert!(!token.is_empty());
    assert!(validate_csrf_token(&token, &token));

    let different_session = Uuid::new_v4();
    let token2 = generate_csrf_token(different_session, secret);
    assert_ne!(token, token2);
    assert!(!validate_csrf_token(&token, &token2));
}

#[test]
fn test_login_rate_limiter() {
    let limiter = LoginRateLimiter::new(3, Duration::from_secs(60));
    let ip: IpAddr = "192.168.1.100".parse().unwrap();
    let username = "admin";

    assert!(limiter.check_rate_limit(ip, username).is_ok());

    limiter.record_failure(ip, username);
    limiter.record_failure(ip, username);
    assert!(limiter.check_rate_limit(ip, username).is_ok());

    limiter.record_failure(ip, username);
    assert!(limiter.check_rate_limit(ip, username).is_err());

    limiter.clear(ip, username);
    assert!(limiter.check_rate_limit(ip, username).is_ok());
}

#[test]
fn test_session_cookie_formatting() {
    let session_id = Uuid::new_v4();
    let cookie = create_session_cookie(session_id);
    assert!(cookie.contains(&format!("session_id={}", session_id)));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Max-Age=86400"));

    let clear_cookie = clear_session_cookie();
    assert!(clear_cookie.contains("session_id="));
    assert!(clear_cookie.contains("Max-Age=0"));
}

#[test]
fn test_top_nav_bar_role_conditional_rendering() {
    use app::auth::AuthUser;
    use app::routes::home::IndexTemplate;
    use askama::Template;

    // 1. Admin role: "Users" link should be present in top nav
    let admin_user = AuthUser {
        id: 1,
        username: "admin".to_string(),
        display_name: "Admin User".to_string(),
        role: UserRole::Admin,
        session_id: Uuid::new_v4(),
        csrf_token: "csrf_token_admin".to_string(),
    };
    let admin_tmpl = IndexTemplate {
        user: admin_user,
        active_automations_count: 0,
        workers_online_count: 0,
        runs_today_count: 0,
        failed_lost_runs_today_count: 0,
    };
    let admin_html = admin_tmpl.render().unwrap();

    assert!(admin_html.contains("<header class=\"top-nav\">"));
    assert!(admin_html.contains("href=\"/automations\""));
    assert!(admin_html.contains("href=\"/schedules\""));
    assert!(admin_html.contains("href=\"/runs\""));
    assert!(admin_html.contains("href=\"/workers\""));
    assert!(admin_html.contains("href=\"/users\""));
    assert!(admin_html.contains("Admin User"));
    assert!(admin_html.contains("csrf_token_admin"));

    // 2. Editor role: "Users" link should NOT be present
    let editor_user = AuthUser {
        id: 2,
        username: "editor".to_string(),
        display_name: "Editor User".to_string(),
        role: UserRole::Editor,
        session_id: Uuid::new_v4(),
        csrf_token: "csrf_token_editor".to_string(),
    };
    let editor_tmpl = IndexTemplate {
        user: editor_user,
        active_automations_count: 0,
        workers_online_count: 0,
        runs_today_count: 0,
        failed_lost_runs_today_count: 0,
    };
    let editor_html = editor_tmpl.render().unwrap();

    assert!(editor_html.contains("<header class=\"top-nav\">"));
    assert!(editor_html.contains("href=\"/automations\""));
    assert!(!editor_html.contains("href=\"/users\""));
    assert!(editor_html.contains("Editor User"));

    // 3. Viewer role: "Users" link should NOT be present
    let viewer_user = AuthUser {
        id: 3,
        username: "viewer".to_string(),
        display_name: "Viewer User".to_string(),
        role: UserRole::Viewer,
        session_id: Uuid::new_v4(),
        csrf_token: "csrf_token_viewer".to_string(),
    };
    let viewer_tmpl = IndexTemplate {
        user: viewer_user,
        active_automations_count: 0,
        workers_online_count: 0,
        runs_today_count: 0,
        failed_lost_runs_today_count: 0,
    };
    let viewer_html = viewer_tmpl.render().unwrap();

    assert!(viewer_html.contains("<header class=\"top-nav\">"));
    assert!(viewer_html.contains("href=\"/automations\""));
    assert!(!viewer_html.contains("href=\"/users\""));
    assert!(viewer_html.contains("Viewer User"));
}

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());
    sqlx::PgPool::connect(&db_url).await.ok()
}

#[tokio::test]
async fn test_unauthenticated_redirect_to_login() {
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

    let state = AppState {
        db: pool,
        s3_client,
        config,
        rate_limiter: LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/", get(get_index_handler))
        .with_state(state);

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/login"
    );
}

#[tokio::test]
async fn test_full_auth_and_password_workflow() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    // Seed test user
    let username = format!("testuser_{}", Uuid::new_v4().simple());
    let initial_password = "Password123!";
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(initial_password.as_bytes(), &salt)
        .unwrap()
        .to_string();

    let user_row = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, $2, 'Test User', 'editor', true)
        RETURNING id
        "#,
    )
    .bind(&username)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let user_id: i64 = user_row.get("id");

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
        rate_limiter: LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/logout", post(post_logout_handler))
        .route(
            "/account/password",
            get(get_password_handler).post(post_password_handler),
        )
        .route("/", get(get_index_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    // 1. Post login with valid credentials
    let login_form = format!("username={}&password={}", username, initial_password);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(login_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/"
    );

    let cookie_header = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(cookie_header.contains("session_id="));

    // 2. Access dashboard with session cookie
    let req = Request::builder()
        .uri("/")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. GET /account/password with session cookie
    let req = Request::builder()
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body_bytes);
    assert!(html.contains("Change Password"));

    // Extract session ID and generate CSRF token
    let session_id = app::auth::session::extract_session_id(&cookie_header).unwrap();
    let csrf_token = generate_csrf_token(session_id, &config.session_secret);

    // 4. POST /account/password with wrong current password
    let pwd_form = format!(
        "csrf_token={}&current_password=WrongPwd!&new_password=NewPassword123!&confirm_password=NewPassword123!",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(pwd_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 5. POST /account/password with correct current password
    let pwd_form = format!(
        "csrf_token={}&current_password={}&new_password=NewPassword123!&confirm_password=NewPassword123!",
        csrf_token, initial_password
    );
    let req = Request::builder()
        .method("POST")
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(pwd_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify audit log row created
    let audit_row = sqlx::query(
        "SELECT count(*) as count FROM audit_log WHERE user_id = $1 AND action = 'password_change'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let audit_count: i64 = audit_row.get("count");
    assert_eq!(audit_count, 1);

    // 6. Logout
    let logout_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri("/logout")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(logout_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Cleanup test user
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await;
}
