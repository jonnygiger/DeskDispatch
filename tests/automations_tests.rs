#![allow(clippy::unwrap_used)]
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::{
    AppState,
    auth::generate_csrf_token,
    config::{Config, SecretString},
    routes::*,
};
use sqlx::PgPool;
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

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect(&db_url)
        .await
        .ok()
}

struct AutomationsCleanupGuard {
    pool: PgPool,
    user_ids: Vec<i64>,
    automation_ids: Vec<i64>,
    bitmap_ids: Vec<i64>,
}

impl Drop for AutomationsCleanupGuard {
    fn drop(&mut self) {
        if self.user_ids.is_empty() && self.automation_ids.is_empty() && self.bitmap_ids.is_empty()
        {
            return;
        }
        let pool = self.pool.clone();
        let user_ids = self.user_ids.clone();
        let automation_ids = self.automation_ids.clone();
        let bitmap_ids = self.bitmap_ids.clone();

        let cleanup = async move {
            if !automation_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM automation_steps WHERE automation_id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ =
                    sqlx::query("DELETE FROM automation_variables WHERE automation_id = ANY($1)")
                        .bind(&automation_ids)
                        .execute(&pool)
                        .await;
                let _ = sqlx::query(
                    "DELETE FROM automation_parameters WHERE automation_id = ANY($1)",
                )
                .bind(&automation_ids)
                .execute(&pool)
                .await;
                let _ = sqlx::query("DELETE FROM automations WHERE id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
            }
            if !bitmap_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM bitmaps WHERE id = ANY($1)")
                    .bind(&bitmap_ids)
                    .execute(&pool)
                    .await;
            }
            if !user_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM sessions WHERE user_id = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM users WHERE id = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
            }
        };

        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(cleanup);
            }
        })
        .join();
    }
}

#[derive(sqlx::FromRow)]
struct StepIdTypeRow {
    id: i64,
    step_type: String,
    label: Option<String>,
    post_delay_ms: i32,
}

#[derive(sqlx::FromRow)]
struct MouseClickDbRow {
    x: Option<i32>,
    y: Option<i32>,
    x_variable_id: Option<i64>,
    #[allow(dead_code)]
    y_variable_id: Option<i64>,
    button: String,
    click_type: String,
}

#[derive(sqlx::FromRow)]
struct FindPixelDbRow {
    x: i32,
    y: i32,
    output_variable_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct TypeTextDbRow {
    text_to_type: String,
    delay_between_keys_ms: i32,
}

#[derive(sqlx::FromRow)]
struct WindowControlDbRow {
    action: String,
    match_pattern: String,
    match_type: String,
    timeout_ms: i64,
}

#[derive(sqlx::FromRow)]
struct BranchDbRow {
    condition_type: String,
    automation_id: i64,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ParamDbRow {
    name: String,
    #[allow(dead_code)]
    param_type: String,
    default_value: String,
    description: String,
}

async fn create_test_user(pool: &PgPool, username: &str, role: &str) -> (i64, String) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, $3) RETURNING id",
    )
    .bind(username)
    .bind(username)
    .bind(role)
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

    (user_id, session_id.to_string())
}

#[tokio::test]
async fn test_automations_crud_and_steps_ordering() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_automations_crud_and_steps_ordering");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/new",
            axum::routing::get(get_new_automation_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler).post(post_automation_edit_handler),
        )
        .route(
            "/automations/{id}/delete",
            axum::routing::get(get_automation_delete_handler).post(post_automation_delete_handler),
        )
        .route(
            "/automations/{id}/steps/new",
            axum::routing::get(get_step_type_picker_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/move-up",
            axum::routing::post(post_move_step_up_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/move-down",
            axum::routing::post(post_move_step_down_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/delete",
            axum::routing::post(post_delete_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("auto_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Test+Automation&description=Sample+desc&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("/automations/"));
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. Fetch automation details
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Add Step 1 (key_press: F5)
    let step1_body = format!(
        "key_combo=F5&label=Refresh&post_delay_seconds=1.0&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(step1_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Add Step 2 (key_press: enter)
    let step2_body = format!(
        "key_combo=enter&label=Confirm&post_delay_seconds=0.5&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(step2_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify steps in DB
    let step_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(step_ids.len(), 2);
    let step1_id = step_ids[0];
    let step2_id = step_ids[1];

    // 4. Move step 2 UP
    let move_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/steps/{}/move-up",
            automation_id, step2_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(move_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify step 2 is now first
    let steps_after_move: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(steps_after_move[0], step2_id);

    // 5. Delete step 1
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/steps/{}/delete",
            automation_id, step1_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(move_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let remaining_steps: Vec<i64> = sqlx::query_scalar("SELECT id FROM automation_steps WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(remaining_steps.len(), 1);

    // 6. Delete automation
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/delete", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(move_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let auto_check: Option<i64> = sqlx::query_scalar("SELECT id FROM automations WHERE id = $1")
        .bind(automation_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(auto_check.is_none());
}

#[tokio::test]
async fn test_mouse_click_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_mouse_click_step_crud_and_validation");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("mc_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Mouse+Click+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. GET new mouse_click step page with query params
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/mouse_click?x=824&y=391",
            automation_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid mouse_click (missing X coordinate)
    let invalid_body = format!(
        "step_type=mouse_click&x_mode=fixed&y_mode=fixed&y=100&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(invalid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST valid mouse_click step
    let valid_body = format!(
        "step_type=mouse_click&label=Click+Submit&x_mode=fixed&x=824&y_mode=fixed&y=391&button=left&click_type=single&post_delay_seconds=0.5&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(valid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify DB insertion
    let step_row: StepIdTypeRow = sqlx::query_as(
        "SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step_row.step_type, "mouse_click");
    assert_eq!(step_row.label.as_deref(), Some("Click Submit"));
    assert_eq!(step_row.post_delay_ms, 500);

    let click_row: MouseClickDbRow = sqlx::query_as("SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
        .bind(step_row.id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(click_row.x, Some(824));
    assert_eq!(click_row.y, Some(391));
    assert_eq!(click_row.button, "left");
    assert_eq!(click_row.click_type, "single");

    // 5. GET edit mouse_click page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, step_row.id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Create variable and update step to variable binding
    let var_id: i64 = sqlx::query_scalar("INSERT INTO automation_variables (automation_id, name, var_type) VALUES ($1, 'target_x', 'int') RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let edit_body = format!(
        "step_type=mouse_click&label=Click+Variable&x_mode=variable&x_variable_id={}&y_mode=fixed&y=400&button=right&click_type=double&post_delay_seconds=1.0&csrf_token={}",
        var_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_row.id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_click_row: MouseClickDbRow = sqlx::query_as("SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
        .bind(step_row.id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(updated_click_row.x, None);
    assert_eq!(updated_click_row.x_variable_id, Some(var_id));
    assert_eq!(updated_click_row.y, Some(400));
    assert_eq!(updated_click_row.button, "right");
    assert_eq!(updated_click_row.click_type, "double");

    // 7. Verify plain-language string rendering in detail page for mouse_click with variable
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Click («target_x», 400) [right, double]"));
}

#[tokio::test]
async fn test_find_pixel_rgb_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_find_pixel_rgb_step_crud_and_validation");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("fp_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Pixel+RGB+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. GET new find_pixel_rgb step page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/find_pixel_rgb?x=100&y=200",
            automation_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid find_pixel_rgb (missing X)
    let invalid_body = format!("step_type=find_pixel_rgb&y=200&csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(invalid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST valid find_pixel_rgb step
    let valid_body = format!(
        "step_type=find_pixel_rgb&label=Sample+Pixel&x=100&y=200&post_delay_seconds=0.5&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(valid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify DB insertion
    let step_row: StepIdTypeRow = sqlx::query_as(
        "SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step_row.step_type, "find_pixel_rgb");
    assert_eq!(step_row.label.as_deref(), Some("Sample Pixel"));
    assert_eq!(step_row.post_delay_ms, 500);

    let fp_row: FindPixelDbRow =
        sqlx::query_as("SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1")
            .bind(step_row.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(fp_row.x, 100);
    assert_eq!(fp_row.y, 200);
    assert_eq!(fp_row.output_variable_id, None);

    // 5. GET edit page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, step_row.id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Create output variable and update step
    let var_id: i64 = sqlx::query_scalar("INSERT INTO automation_variables (automation_id, name, var_type) VALUES ($1, 'bg_color', 'color') RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let edit_body = format!(
        "step_type=find_pixel_rgb&label=Updated+Pixel&x=150&y=250&output_variable_id={}&post_delay_seconds=1.0&csrf_token={}",
        var_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_row.id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_fp_row: FindPixelDbRow =
        sqlx::query_as("SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1")
            .bind(step_row.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(updated_fp_row.x, 150);
    assert_eq!(updated_fp_row.y, 250);
    assert_eq!(updated_fp_row.output_variable_id, Some(var_id));

    // 7. Verify plain-language string rendering in detail page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Read pixel at (150, 250) → store as «bg_color»"));
}

#[tokio::test]
async fn test_step_type_picker_interface() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_step_type_picker_interface");
        return;
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
            "/automations/{id}/steps/new",
            axum::routing::get(get_step_type_picker_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("picker_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let _guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let req = Request::builder()
        .method("GET")
        .uri("/automations/42/steps/new")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body_str.contains("Select Step Type"));
    assert!(body_str.contains("/automations/42/steps/new/key_press"));
    assert!(body_str.contains("/automations/42/steps/new/type_text"));
    assert!(body_str.contains("/automations/42/steps/new/window_control"));
    assert!(body_str.contains("/automations/42/steps/new/mouse_click"));
    assert!(body_str.contains("/automations/42/steps/new/find_pixel_rgb"));
    assert!(body_str.contains("Key Press"));
    assert!(body_str.contains("Type Text"));
    assert!(body_str.contains("Window Control"));
    assert!(body_str.contains("Mouse Click"));
    assert!(body_str.contains("Mouse Scroll"));
    assert!(body_str.contains("Find Pixel RGB"));
    assert!(body_str.contains("Find Bitmap"));
    assert!(body_str.contains("Branch"));
}

#[tokio::test]
async fn test_type_text_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_type_text_step_crud_and_validation");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("tt_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Type+Text+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. GET new type_text step page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/type_text",
            automation_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid type_text (empty text_to_type)
    let invalid_body = format!(
        "step_type=type_text&text_to_type=&delay_between_keys_ms=50&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(invalid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST valid type_text step
    let valid_body = format!(
        "step_type=type_text&label=Type+Greeting&text_to_type=Hello+World&delay_between_keys_ms=50&post_delay_seconds=0.5&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(valid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify DB insertion
    let step_row: StepIdTypeRow = sqlx::query_as(
        "SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step_row.step_type, "type_text");
    assert_eq!(step_row.label.as_deref(), Some("Type Greeting"));
    assert_eq!(step_row.post_delay_ms, 500);

    let tt_row: TypeTextDbRow = sqlx::query_as(
        "SELECT text_to_type, delay_between_keys_ms FROM step_type_texts WHERE step_id = $1",
    )
    .bind(step_row.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(tt_row.text_to_type, "Hello World");
    assert_eq!(tt_row.delay_between_keys_ms, 50);

    // 5. GET edit page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, step_row.id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Update step
    let edit_body = format!(
        "step_type=type_text&label=Updated+Greeting&text_to_type=Hello+DeskDispatch&delay_between_keys_ms=100&post_delay_seconds=1.0&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_row.id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_tt_row: TypeTextDbRow = sqlx::query_as(
        "SELECT text_to_type, delay_between_keys_ms FROM step_type_texts WHERE step_id = $1",
    )
    .bind(step_row.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(updated_tt_row.text_to_type, "Hello DeskDispatch");
    assert_eq!(updated_tt_row.delay_between_keys_ms, 100);

    // 7. Verify plain-language summary rendering in automation detail page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Type \"Hello DeskDispatch\" (100ms key delay)"));
}

#[tokio::test]
async fn test_window_control_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_window_control_step_crud_and_validation");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("wc_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Window+Control+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. GET new window_control step page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/window_control",
            automation_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid window_control (empty match_pattern)
    let invalid_body = format!(
        "step_type=window_control&action=activate&match_pattern=&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(invalid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST valid window_control step
    let valid_body = format!(
        "step_type=window_control&label=Focus+Firefox&action=activate&match_pattern=Mozilla+Firefox&match_type=contains&timeout_seconds=5.0&post_delay_seconds=0.5&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(valid_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify DB insertion
    let step_row: StepIdTypeRow = sqlx::query_as(
        "SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step_row.step_type, "window_control");
    assert_eq!(step_row.label.as_deref(), Some("Focus Firefox"));
    assert_eq!(step_row.post_delay_ms, 500);

    let wc_row: WindowControlDbRow = sqlx::query_as(
        "SELECT action, match_pattern, match_type, timeout_ms FROM step_window_controls WHERE step_id = $1",
    )
    .bind(step_row.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(wc_row.action, "activate");
    assert_eq!(wc_row.match_pattern, "Mozilla Firefox");
    assert_eq!(wc_row.match_type, "contains");
    assert_eq!(wc_row.timeout_ms, 5000);

    // 5. GET edit page
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, step_row.id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Update step
    let edit_body = format!(
        "step_type=window_control&label=Wait+for+Calc&action=wait&match_pattern=Calculator&match_type=exact&timeout_seconds=10.0&post_delay_seconds=1.0&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_row.id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_wc_row: WindowControlDbRow = sqlx::query_as(
        "SELECT action, match_pattern, match_type, timeout_ms FROM step_window_controls WHERE step_id = $1",
    )
    .bind(step_row.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(updated_wc_row.action, "wait");
    assert_eq!(updated_wc_row.match_pattern, "Calculator");
    assert_eq!(updated_wc_row.match_type, "exact");
    assert_eq!(updated_wc_row.timeout_ms, 10000);

    // 7. Verify plain-language summary rendering in automation detail page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Wait for window matching \"Calculator\" (exact) [timeout 10.0s]"));
}

#[tokio::test]
async fn test_branch_step_target_selectors_and_foreign_keys() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_branch_step_target_selectors_and_foreign_keys"
        );
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("branch_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Branch+Target+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. Add Step 1 (key_press)
    let step1_body = format!("key_combo=F5&label=Step+Alpha&csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(step1_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Add Step 2 (key_press)
    let step2_body = format!("key_combo=enter&label=Step+Beta&csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(step2_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Fetch step IDs
    let step_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(step_ids.len(), 2);
    let step1_id = step_ids[0];
    let step2_id = step_ids[1];

    // 3. GET /automations/{id}/steps/new/branch and check target step dropdown options
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/new/branch", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body_str.contains("Step 1 (Step Alpha)"));
    assert!(body_str.contains("Step 2 (Step Beta)"));

    // 4. POST create branch step targeting step1_id (match) and step2_id (no_match)
    let branch_create_body = format!(
        "step_type=branch&label=Check+Pixel&condition_type=pixel_rgb&x=100&y=200&expected_r=10&expected_g=20&expected_b=30&tolerance=5&on_match_step_id={}&on_no_match_step_id={}&csrf_token={}",
        step1_id, step2_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(branch_create_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Fetch branch step ID
    let steps_all: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(steps_all.len(), 3);
    let branch_step_id = steps_all[2];

    // Verify foreign keys in step_branches
    let branch_row: BranchDbRow = sqlx::query_as("SELECT condition_type, automation_id, on_match_step_id, on_no_match_step_id FROM step_branches WHERE step_id = $1")
        .bind(branch_step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(branch_row.condition_type, "pixel_rgb");
    assert_eq!(branch_row.automation_id, automation_id);
    assert_eq!(branch_row.on_match_step_id, Some(step1_id));
    assert_eq!(branch_row.on_no_match_step_id, Some(step2_id));

    // 5. GET edit branch step page and check selected targets
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, branch_step_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. POST edit branch step swapping target foreign keys
    let branch_edit_body = format!(
        "step_type=branch&label=Check+Pixel+Updated&condition_type=pixel_rgb&x=100&y=200&expected_r=10&expected_g=20&expected_b=30&tolerance=5&on_match_step_id={}&on_no_match_step_id={}&csrf_token={}",
        step2_id, step1_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/steps/{}",
            automation_id, branch_step_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(branch_edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated foreign keys in step_branches
    let updated_branch_row: BranchDbRow = sqlx::query_as(
        "SELECT condition_type, automation_id, on_match_step_id, on_no_match_step_id FROM step_branches WHERE step_id = $1",
    )
    .bind(branch_step_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(updated_branch_row.on_match_step_id, Some(step2_id));
    assert_eq!(updated_branch_row.on_no_match_step_id, Some(step1_id));

    // 7. Verify plain-language branch step description rendering in automation detail view
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("BRANCH: if pixel at (100, 200) ≈ RGB(10,20,30) ±5 → go to Step 2 (Step Beta), else → go to Step 1 (Step Alpha)"));
}

#[tokio::test]
async fn test_automation_parameters_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_automation_parameters_crud_and_validation");
        return;
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
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}/parameters",
            axum::routing::get(get_automation_parameters_handler)
                .post(post_create_automation_parameter_handler),
        )
        .route(
            "/automations/{id}/parameters/{pid}",
            axum::routing::post(post_update_automation_parameter_handler),
        )
        .route(
            "/automations/{id}/parameters/{pid}/delete",
            axum::routing::post(post_delete_automation_parameter_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("param_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!(
        "name=Param+Test+Automation&description=Test&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 2. GET /automations/{id}/parameters
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/parameters", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid parameter (invalid default_value for int)
    let invalid_param_body = format!(
        "name=click_tolerance&param_type=int&default_value=invalid_num&description=Tolerance&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/parameters", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(invalid_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST valid parameter
    let valid_param_body = format!(
        "name=click_tolerance&param_type=int&default_value=10&description=Tolerance&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/parameters", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(valid_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Check parameter in DB
    let param_id: i64 = sqlx::query_scalar("SELECT id FROM automation_parameters WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let param_row: ParamDbRow = sqlx::query_as("SELECT name, param_type, default_value, description FROM automation_parameters WHERE id = $1")
        .bind(param_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(param_row.name, "click_tolerance");
    assert_eq!(param_row.default_value, "10");

    // 5. POST update parameter
    let update_param_body = format!(
        "name=click_tolerance&param_type=int&default_value=15&description=Updated+Tolerance&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/parameters/{}",
            automation_id, param_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(update_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let updated_row: ParamDbRow =
        sqlx::query_as("SELECT name, param_type, default_value, description FROM automation_parameters WHERE id = $1")
            .bind(param_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(updated_row.default_value, "15");
    assert_eq!(updated_row.description, "Updated Tolerance");

    // 6. POST delete parameter
    let delete_param_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/parameters/{}/delete",
            automation_id, param_id
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(delete_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM automation_parameters WHERE automation_id = $1")
            .bind(automation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_e2e_admin_password_change_automation_bitmap_step_flow() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_e2e_admin_password_change_automation_bitmap_step_flow"
        );
        return;
    };

    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        Some("http://localhost:9000"),
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
            "/login",
            axum::routing::get(get_login_handler).post(post_login_handler),
        )
        .route(
            "/account/password",
            axum::routing::get(get_password_handler).post(post_password_handler),
        )
        .route(
            "/automations",
            axum::routing::get(get_automations_handler).post(post_automations_handler),
        )
        .route(
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/bitmaps",
            axum::routing::get(get_bitmaps_handler).post(post_bitmaps_handler),
        )
        .route(
            "/bitmaps/commit",
            axum::routing::get(get_bitmap_commit_handler).post(bitmaps::post_bitmap_commit_handler),
        )
        .route(
            "/automations/{id}/steps/new/{step_type}",
            axum::routing::get(get_new_step_handler),
        )
        .route(
            "/automations/{id}/steps",
            axum::routing::post(post_create_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}/edit",
            axum::routing::get(get_edit_step_handler),
        )
        .route(
            "/automations/{id}/steps/{sid}",
            axum::routing::post(post_edit_step_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    // 1. Seed admin user
    let username = format!("admin_e2e_{}", uuid::Uuid::new_v4().simple());
    let old_password = "InitialAdminPassword123!";
    let new_password = "NewUpdatedAdminPassword456!";

    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(old_password.as_bytes())
        .unwrap()
        .to_string();

    let admin_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role, is_active) VALUES ($1, $2, 'E2E Admin', 'admin', true) RETURNING id",
    )
    .bind(&username)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![admin_id],
        automation_ids: vec![],
        bitmap_ids: vec![],
    };

    // Admin logs in via POST /login
    let login_form = format!("username={}&password={}", username, old_password);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(login_form))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let cookie_header = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let session_id = deskdispatch::auth::session::extract_session_id(&cookie_header).unwrap();
    let csrf_token = generate_csrf_token(session_id, config.session_secret.expose_secret());

    // 2. Admin changes password via POST /account/password
    let change_pwd_form = format!(
        "csrf_token={}&current_password={}&new_password={}&confirm_password={}",
        csrf_token, old_password, new_password, new_password
    );
    let req = Request::builder()
        .method("POST")
        .uri("/account/password")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(change_pwd_form))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verify updated password hash in DB
    let updated_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let parsed_hash = PasswordHash::new(&updated_hash).unwrap();
    assert!(
        argon2
            .verify_password(new_password.as_bytes(), &parsed_hash)
            .is_ok()
    );

    // 3. Admin creates an automation via POST /automations
    let auto_name = format!("E2E Admin Automation {}", uuid::Uuid::new_v4().simple());
    let auto_name_encoded = deskdispatch::storage::url_encode_component(&auto_name);
    let create_auto_form = format!(
        "csrf_token={}&name={}&description=Created+during+E2E+test",
        csrf_token, auto_name_encoded
    );
    let req = Request::builder()
        .method("POST")
        .uri("/automations")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_auto_form))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let automation_id: i64 = location
        .trim_start_matches("/automations/")
        .parse()
        .unwrap();
    guard.automation_ids.push(automation_id);

    // 4. Admin uploads a reference bitmap
    let test_bitmap_name = format!("E2E Target Logo {}", uuid::Uuid::new_v4().simple());
    let bitmap_key = format!(
        "bitmaps/e2e_{}_{}.png",
        admin_id,
        uuid::Uuid::new_v4().simple()
    );

    // Upload 15x20 PNG image bytes to S3
    let blue_pixels = [0, 128, 255].repeat(15 * 20);
    let png_bytes = deskdispatch::routes::bitmaps::encode_png(15, 20, &blue_pixels);

    let _ = state
        .storage_service()
        .put_object(&bitmap_key, png_bytes, "image/png")
        .await;

    // Commit reference bitmap via POST /bitmaps/commit
    let test_bitmap_name_encoded = deskdispatch::storage::url_encode_component(&test_bitmap_name);
    let commit_form = format!(
        "csrf_token={}&key={}&name={}",
        csrf_token, bitmap_key, test_bitmap_name_encoded
    );
    let req = Request::builder()
        .method("POST")
        .uri("/bitmaps/commit")
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(commit_form))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Obtain uploaded bitmap record ID from DB
    let bitmap_id: i64 = sqlx::query_scalar("SELECT id FROM bitmaps WHERE object_storage_key = $1")
        .bind(&bitmap_key)
        .fetch_one(&pool)
        .await
        .unwrap();
    guard.bitmap_ids.push(bitmap_id);

    // 5. Admin creates a step using the bitmap via POST /automations/{id}/steps
    let create_step_form = format!(
        "step_type=find_bitmap&label=Locate+Target+Logo&reference_bitmap_id={}&match_threshold=0.95&post_delay_seconds=1.0&csrf_token={}",
        bitmap_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(create_step_form))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let step_id: i64 = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 AND step_type = 'find_bitmap'",
    )
    .bind(automation_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 6. In the image selection screen, verify presented image matches up with uploaded bitmap
    // Test A: GET /automations/{id}/steps/new/find_bitmap renders reference_bitmap_id options
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/find_bitmap",
            automation_id
        ))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html_new = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(
        html_new.contains(&test_bitmap_name),
        "Uploaded bitmap name missing in step creation screen"
    );
    assert!(
        html_new.contains(&format!("value=\"{}\"", bitmap_id)),
        "Uploaded bitmap ID missing in option list"
    );
    assert!(
        html_new.contains("15x20px"),
        "Uploaded bitmap dimensions missing in option text"
    );

    // Test B: GET /automations/{id}/steps/{sid}/edit presents the uploaded bitmap selected
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/{}/edit",
            automation_id, step_id
        ))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html_edit = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(
        html_edit.contains(&test_bitmap_name),
        "Uploaded bitmap name missing in step edit screen"
    );
    let expected_option = format!("<option value=\"{}\" selected>", bitmap_id);
    assert!(
        html_edit.contains(&expected_option),
        "Uploaded bitmap option is not selected in step edit screen. HTML excerpt: {}",
        html_edit
    );
}

#[tokio::test]
async fn test_ocr_extract_step_crud_and_task_output() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_ocr_extract_step_crud_and_task_output");
        return;
    };

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ('ocr_editor', 'hash', 'Test', 'editor') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let session_uuid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let cookie_header = format!("__Host-session_id={}", session_uuid);

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

    let app = deskdispatch::routes::automations::router().with_state(state);

    let automation_id: i64 = sqlx::query_scalar("INSERT INTO automations (name, description, status, created_by) VALUES ('OCR Flow', 'Test OCR', 'draft', $1) RETURNING id")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let _guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![user_id],
        automation_ids: vec![automation_id],
        bitmap_ids: vec![],
    };

    let csrf = generate_csrf_token(session_uuid, config.session_secret.expose_secret());

    // 1. GET /automations/{id}/steps/new/ocr_extract
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/ocr_extract",
            automation_id
        ))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. POST /automations/{id}/steps creating ocr_extract step
    let form_body = format!(
        "csrf_token={}&step_type=ocr_extract&label=Read+Header&search_x=10&search_y=20&search_width=200&search_height=50&set_as_task_output=true&post_delay_seconds=0.5",
        csrf
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 3. GET /automations/{id} and verify OCR step is listed
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(html.contains("Read Header"));
    assert!(html.contains("ocr_extract"));
    assert!(html.contains("Extract text via OCR"));
}

#[tokio::test]
async fn test_clipboard_control_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_clipboard_control_step_crud_and_validation"
        );
        return;
    };

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ('clip_editor', 'hash', 'Test', 'editor') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let session_uuid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let cookie_header = format!("__Host-session_id={}", session_uuid);

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

    let app = deskdispatch::routes::automations::router().with_state(state);

    let automation_id: i64 = sqlx::query_scalar("INSERT INTO automations (name, description, status, created_by) VALUES ('Clipboard Flow', 'Test Clipboard', 'draft', $1) RETURNING id")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let _guard = AutomationsCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![user_id],
        automation_ids: vec![automation_id],
        bitmap_ids: vec![],
    };

    let csrf = generate_csrf_token(session_uuid, config.session_secret.expose_secret());

    // 1. GET /automations/{id}/steps/new/clipboard_control
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/automations/{}/steps/new/clipboard_control",
            automation_id
        ))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. POST /automations/{id}/steps creating set_text clipboard step
    let form_body = format!(
        "csrf_token={}&step_type=clipboard_control&label=Set+System+Clipboard&action=set_text&text_mode=literal&text_to_set=Hello+DeskDispatch&post_delay_seconds=0.5",
        csrf
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps", automation_id))
        .header(header::COOKIE, &cookie_header)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 3. GET /automations/{id} and verify clipboard step is listed
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(html.contains("Set System Clipboard"));
    assert!(html.contains("clipboard_control"));
    assert!(html.contains("Set clipboard to"));
}
