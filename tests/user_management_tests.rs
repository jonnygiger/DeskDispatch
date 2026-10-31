#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    middleware,
    routing::{get, post},
};
use chrono::Utc;
use deskdispatch::AppState;
use deskdispatch::auth::{UserRole, generate_csrf_token};
use deskdispatch::config::{Config, SecretString};
use deskdispatch::routes::{
    UserListItem, get_edit_user_handler, get_new_user_handler, get_password_handler,
    get_reset_password_handler, get_users_handler, post_create_user_handler,
    post_deactivate_user_handler, post_edit_user_handler, post_login_handler,
    post_password_handler, post_reset_password_handler,
};
use tower::ServiceExt;
use uuid::Uuid;

struct UserManagementCleanupGuard {
    pool: sqlx::PgPool,
    user_ids: Vec<i64>,
}

impl Drop for UserManagementCleanupGuard {
    fn drop(&mut self) {
        if self.user_ids.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        let user_ids = self.user_ids.clone();

        let cleanup = async move {
            let _ = sqlx::query("DELETE FROM users WHERE id = ANY($1)")
                .bind(&user_ids)
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

#[derive(sqlx::FromRow)]
struct TestUserRow {
    id: i64,
    display_name: String,
    must_change_password: bool,
}

async fn create_test_user(
    pool: &sqlx::PgPool,
    username: &str,
    role: &str,
    must_change_password: bool,
) -> (i64, String) {
    create_test_user_with_hash(pool, username, role, must_change_password, "hash").await
}

async fn create_test_user_with_hash(
    pool: &sqlx::PgPool,
    username: &str,
    role: &str,
    must_change_password: bool,
    password_hash: &str,
) -> (i64, String) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role, is_active, must_change_password) VALUES ($1, $2, $3, $4, true, $5) RETURNING id",
    )
    .bind(username)
    .bind(password_hash)
    .bind(username)
    .bind(role)
    .bind(must_change_password)
    .fetch_one(pool)
    .await
    .unwrap();

    let session_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();

    (user_id, format!("session_id={}", session_id))
}

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

fn build_test_app(pool: sqlx::PgPool, config: Config) -> Router {
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
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    Router::new()
        .route(
            "/login",
            get(deskdispatch::routes::get_login_handler).post(post_login_handler),
        )
        .route(
            "/account/password",
            get(get_password_handler).post(post_password_handler),
        )
        .route(
            "/users",
            get(get_users_handler).post(post_create_user_handler),
        )
        .route("/users/new", get(get_new_user_handler))
        .route("/users/{id}", post(post_edit_user_handler))
        .route("/users/{id}/edit", get(get_edit_user_handler))
        .route("/users/{id}/deactivate", post(post_deactivate_user_handler))
        .route(
            "/users/{id}/reset-password",
            get(get_reset_password_handler).post(post_reset_password_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state)
}

#[tokio::test]
async fn test_non_admin_forbidden_from_user_management() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = create_test_config();

    // Directly seed editor user and session without slow Argon2 password hashing or HTTP login roundtrips
    let editor_username = format!("editor_{}", Uuid::new_v4().simple());
    let (editor_id, cookie_header) =
        create_test_user(&pool, &editor_username, "editor", false).await;

    let _cleanup_guard = UserManagementCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![editor_id],
    };

    let app = build_test_app(pool.clone(), config);

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
}

#[tokio::test]
async fn test_full_user_crud_and_last_admin_protection() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = create_test_config();

    // Directly seed admin user and session without slow Argon2 password hashing or HTTP login roundtrips
    let admin_username = format!("admin_{}", Uuid::new_v4().simple());
    let (admin_id, cookie_header) = create_test_user(&pool, &admin_username, "admin", false).await;

    let mut cleanup_guard = UserManagementCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
    };

    let app = build_test_app(pool.clone(), config.clone());

    let session_id = deskdispatch::auth::session::extract_session_id(&cookie_header).unwrap();
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

    // Retrieve created user using strongly-typed FromRow struct
    let new_user: TestUserRow = sqlx::query_as(
        "SELECT id, display_name, must_change_password FROM users WHERE username = $1",
    )
    .bind(&new_username)
    .fetch_one(&pool)
    .await
    .unwrap();

    cleanup_guard.user_ids.push(new_user.id);
    assert!(new_user.must_change_password);

    // 4. Edit user via POST /users/{id}
    let edit_form = format!(
        "csrf_token={}&display_name=Updated+Display+Name&role=editor&is_active=true&must_change_password=false",
        csrf_token
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}", new_user.id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify updated display_name and must_change_password
    let updated_user: TestUserRow =
        sqlx::query_as("SELECT id, display_name, must_change_password FROM users WHERE id = $1")
            .bind(new_user.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(updated_user.display_name, "Updated Display Name");
    assert!(!updated_user.must_change_password);

    // 5. Reset user password via POST /users/{id}/reset-password
    let reset_form = format!(
        "csrf_token={}&new_password=ResetPwd123!&confirm_password=ResetPwd123!&must_change_password=true",
        csrf_token
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}/reset-password", new_user.id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(reset_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    // Verify must_change_password set back to true
    let must_change_after_reset: bool =
        sqlx::query_scalar("SELECT must_change_password FROM users WHERE id = $1")
            .bind(new_user.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(must_change_after_reset);

    // 6. Deactivate user via POST /users/{id}/deactivate
    let deact_form = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/users/{}/deactivate", new_user.id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(deact_form))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let deactivated_is_active: bool =
        sqlx::query_scalar("SELECT is_active FROM users WHERE id = $1")
            .bind(new_user.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(!deactivated_is_active);

    // 7. Test protection of last active admin:
    // Check current active admin count in database
    let total_active_admins: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = true")
            .fetch_one(&pool)
            .await
            .unwrap();

    if total_active_admins > 1 {
        // Query other active admin IDs and temporarily deactivate them on pool so Axum handlers see active_admins == 1
        let other_active_admin_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM users WHERE role = 'admin' AND is_active = true AND id != $1",
        )
        .bind(admin_id)
        .fetch_all(&pool)
        .await
        .unwrap();

        struct AdminRestorationGuard {
            pool: sqlx::PgPool,
            deactivated_admin_ids: Vec<i64>,
        }

        impl Drop for AdminRestorationGuard {
            fn drop(&mut self) {
                if self.deactivated_admin_ids.is_empty() {
                    return;
                }
                let pool = self.pool.clone();
                let ids = self.deactivated_admin_ids.clone();

                let restore = async move {
                    let _ = sqlx::query("UPDATE users SET is_active = true WHERE id = ANY($1)")
                        .bind(&ids)
                        .execute(&pool)
                        .await;
                };

                let _ = std::thread::spawn(move || {
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        rt.block_on(restore);
                    }
                })
                .join();
            }
        }

        let restoration_guard = AdminRestorationGuard {
            pool: pool.clone(),
            deactivated_admin_ids: other_active_admin_ids.clone(),
        };

        if !other_active_admin_ids.is_empty() {
            sqlx::query("UPDATE users SET is_active = false WHERE id = ANY($1)")
                .bind(&other_active_admin_ids)
                .execute(&pool)
                .await
                .unwrap();
        }

        // Attempting to deactivate admin_id when active_admins == 1 returns 400 BAD_REQUEST
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

        drop(restoration_guard);
    } else {
        // admin_id is already the sole active admin: deactivating it directly returns 400 Bad Request
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
    }
}

#[tokio::test]
async fn test_forced_password_change_redirect() {
    use argon2::{Argon2, password_hash::PasswordHasher};

    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    let config = create_test_config();

    // Directly seed user with must_change_password = true and active session
    let pwd = "InitialPwd123!";
    let hash = Argon2::default()
        .hash_password(pwd.as_bytes())
        .unwrap()
        .to_string();

    let username = format!("force_pwd_{}", Uuid::new_v4().simple());
    let (user_id, cookie_header) =
        create_test_user_with_hash(&pool, &username, "editor", true, &hash).await;

    let _cleanup_guard = UserManagementCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![user_id],
    };

    let app = build_test_app(pool.clone(), config.clone());

    // Attempting to access /users should redirect to /account/password
    let req = Request::builder()
        .uri("/users")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/account/password"
    );

    // Accessing /account/password should succeed
    let req = Request::builder()
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update password via POST /account/password
    let session_id = deskdispatch::auth::session::extract_session_id(&cookie_header).unwrap();
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
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(!must_change);
}
