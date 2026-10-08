use app::auth::{
    generate_csrf_token, UserRole,
};
use secrecy::ExposeSecret;
use app::config::Config;
use app::routes::{
    get_edit_user_handler, get_new_user_handler, get_password_handler, get_reset_password_handler,
    get_users_handler, post_create_user_handler, post_deactivate_user_handler,
    post_edit_user_handler, post_login_handler, post_password_handler,
    post_reset_password_handler, UserListItem,
};
use app::AppState;
use argon2::{
    password_hash::PasswordHasher,
    Argon2,
};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn test_user_list_item_helpers() {
    let now = Utc::now();
    let item = UserListItem {
        id: 1,
        username: "jdoe".to_string(),
        display_name: "John Doe".to_string(),
        role: UserRole::Admin,
        is_active: true,
        must_change_password: true,
        created_at: now,
        last_login_at: None,
    };

    assert_eq!(item.status_badge_class(), "badge-success");
    assert_eq!(item.role_badge_class(), "badge-primary");
    assert_eq!(item.formatted_last_login(), "Never");
    assert!(!item.formatted_created_at().is_empty());

    let inactive_editor = UserListItem {
        id: 2,
        username: "asmith".to_string(),
        display_name: "Alice Smith".to_string(),
        role: UserRole::Editor,
        is_active: false,
        must_change_password: false,
        created_at: now,
        last_login_at: Some(now),
    };

    assert_eq!(inactive_editor.status_badge_class(), "badge-danger");
    assert_eq!(inactive_editor.role_badge_class(), "badge-neutral");
    assert_ne!(inactive_editor.formatted_last_login(), "Never");
}

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());
    sqlx::PgPool::connect(&db_url).await.ok()
}

fn build_test_app(pool: sqlx::PgPool, config: Config) -> Router {
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
        rate_limiter: app::auth::LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
    };

    Router::new()
        .route("/login", get(app::routes::get_login_handler).post(post_login_handler))
        .route("/account/password", get(get_password_handler).post(post_password_handler))
        .route("/users", get(get_users_handler).post(post_create_user_handler))
        .route("/users/new", get(get_new_user_handler))
        .route("/users/{id}", post(post_edit_user_handler))
        .route("/users/{id}/edit", get(get_edit_user_handler))
        .route("/users/{id}/deactivate", post(post_deactivate_user_handler))
        .route("/users/{id}/reset-password", get(get_reset_password_handler).post(post_reset_password_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state)
}

#[tokio::test]
async fn test_non_admin_forbidden_from_user_management() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = Config::from_env().unwrap();

    // Create an editor user
    let editor_username = format!("editor_{}", Uuid::new_v4().simple());
    let pwd = "Password123!";
    let hash = Argon2::default().hash_password(pwd.as_bytes()).unwrap().to_string();

    let editor_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role, is_active) VALUES ($1, $2, 'Editor', 'editor', true) RETURNING id",
    )
    .bind(&editor_username)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = build_test_app(pool.clone(), config);

    // Login as editor
    let login_form = format!("username={}&password={}", editor_username, pwd);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(login_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let cookie_header = response.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();

    // Editor trying to GET /users should receive 403 FORBIDDEN
    let req = Request::builder()
        .uri("/users")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Editor trying to GET /users/new should receive 403 FORBIDDEN
    let req = Request::builder()
        .uri("/users/new")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Clean up test editor
    let _ = sqlx::query("DELETE FROM users WHERE id = $1").bind(editor_id).execute(&pool).await;
}

#[tokio::test]
async fn test_full_user_crud_and_last_admin_protection() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = Config::from_env().unwrap();

    // Create an initial admin user for the test
    let admin_username = format!("admin_{}", Uuid::new_v4().simple());
    let admin_pwd = "AdminPassword123!";
    let admin_hash = Argon2::default().hash_password(admin_pwd.as_bytes()).unwrap().to_string();

    let admin_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role, is_active) VALUES ($1, $2, 'Admin User', 'admin', true) RETURNING id",
    )
    .bind(&admin_username)
    .bind(&admin_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = build_test_app(pool.clone(), config.clone());

    // 1. Login as Admin
    let login_form = format!("username={}&password={}", admin_username, admin_pwd);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(login_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let cookie_header = response.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    let session_id = app::auth::session::extract_session_id(&cookie_header).unwrap();
    let csrf_token = generate_csrf_token(session_id, config.session_secret.expose_secret());

    // 2. GET /users list
    let req = Request::builder()
        .uri("/users")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Create a new user via POST /users
    let new_username = format!("newuser_{}", Uuid::new_v4().simple());
    let create_form = format!(
        "csrf_token={}&username={}&display_name=New+User&role=editor&password=NewUserPwd123!&confirm_password=NewUserPwd123!&must_change_password=true",
        csrf_token, new_username
    );

    let req = Request::builder()
        .method("POST")
        .uri("/users")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/users");

    // Retrieve created user
    let new_user_row = sqlx::query("SELECT id, must_change_password, is_active FROM users WHERE username = $1")
        .bind(&new_username)
        .fetch_one(&pool)
        .await
        .unwrap();

    let new_user_id: i64 = new_user_row.get("id");
    let must_change: bool = new_user_row.get("must_change_password");
    assert!(must_change);

    // 4. Edit user via POST /users/{id}
    let edit_form = format!(
        "csrf_token={}&display_name=Updated+Display+Name&role=editor&is_active=true&must_change_password=false",
        csrf_token
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}", new_user_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify updated display_name and must_change_password
    let updated_row = sqlx::query("SELECT display_name, must_change_password FROM users WHERE id = $1")
        .bind(new_user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(updated_row.get::<String, _>("display_name"), "Updated Display Name");
    assert!(!updated_row.get::<bool, _>("must_change_password"));

    // 5. Reset user password via POST /users/{id}/reset-password
    let reset_form = format!(
        "csrf_token={}&new_password=ResetPwd123!&confirm_password=ResetPwd123!&must_change_password=true",
        csrf_token
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}/reset-password", new_user_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(reset_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify must_change_password set back to true
    let reset_row = sqlx::query("SELECT must_change_password FROM users WHERE id = $1")
        .bind(new_user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(reset_row.get::<bool, _>("must_change_password"));

    // 6. Deactivate user via POST /users/{id}/deactivate
    let deact_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}/deactivate", new_user_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(deact_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let deactivated_is_active: bool = sqlx::query_scalar("SELECT is_active FROM users WHERE id = $1")
        .bind(new_user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(!deactivated_is_active);

    // 7. Test protection of last active admin:
    // If we temporarily deactivate all other admin accounts except admin_id
    let _ = sqlx::query("UPDATE users SET is_active = false WHERE role = 'admin' AND id != $1")
        .bind(admin_id)
        .execute(&pool)
        .await;

    // Attempting to deactivate admin_id should be rejected
    let deact_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}/deactivate", admin_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(deact_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Clean up created users
    let _ = sqlx::query("DELETE FROM users WHERE id IN ($1, $2)")
        .bind(admin_id)
        .bind(new_user_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn test_forced_password_change_redirect() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = Config::from_env().unwrap();

    // Create user with must_change_password = true
    let username = format!("force_pwd_{}", Uuid::new_v4().simple());
    let pwd = "InitialPwd123!";
    let hash = Argon2::default().hash_password(pwd.as_bytes()).unwrap().to_string();

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role, is_active, must_change_password) VALUES ($1, $2, 'Force Pwd User', 'editor', true, true) RETURNING id",
    )
    .bind(&username)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = build_test_app(pool.clone(), config.clone());

    // Login as forced password user
    let login_form = format!("username={}&password={}", username, pwd);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(login_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let cookie_header = response.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();

    // Attempting to access /users should redirect to /account/password
    let req = Request::builder()
        .uri("/users")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/account/password");

    // Accessing /account/password should succeed
    let req = Request::builder()
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update password via POST /account/password
    let session_id = app::auth::session::extract_session_id(&cookie_header).unwrap();
    let csrf_token = generate_csrf_token(session_id, config.session_secret.expose_secret());

    let pwd_form = format!(
        "csrf_token={}&current_password={}&new_password=BrandNewPwd123!&confirm_password=BrandNewPwd123!",
        csrf_token, pwd
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

    // Verify must_change_password is now false in DB
    let must_change: bool = sqlx::query_scalar("SELECT must_change_password FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(!must_change);

    // Clean up
    let _ = sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await;
}
