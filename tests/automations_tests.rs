use app::{
    config::Config,
    routes::*,
    AppState,
};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use sqlx::PgPool;
use tower::ServiceExt;

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());
    PgPool::connect(&db_url).await.ok()
}

async fn create_test_user(pool: &PgPool, username: &str, role: &str) -> (i64, String) {
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, $3) RETURNING id",
    )
    .bind(username)
    .bind(username)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap();

    let user_id: i64 = sqlx::Row::get(&row, "id");

    let session_row = sqlx::query(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();

    let session_id: uuid::Uuid = sqlx::Row::get(&session_row, "id");
    (user_id, session_id.to_string())
}

#[tokio::test]
async fn test_automations_crud_and_steps_ordering() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_automations_crud_and_steps_ordering");
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
        db: pool.clone(),
        s3_client,
        config: config.clone(),
        rate_limiter: app::auth::LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/automations", axum::routing::get(get_automations_handler).post(post_automations_handler))
        .route("/automations/new", axum::routing::get(get_new_automation_handler))
        .route("/automations/{id}", axum::routing::get(get_automation_detail_handler).post(post_automation_edit_handler))
        .route("/automations/{id}/delete", axum::routing::get(get_automation_delete_handler).post(post_automation_delete_handler))
        .route("/automations/{id}/steps/new", axum::routing::get(get_step_type_picker_handler))
        .route("/automations/{id}/steps/new/key_press", axum::routing::get(get_new_key_press_step_handler))
        .route("/automations/{id}/steps", axum::routing::post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", axum::routing::get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", axum::routing::post(post_edit_step_handler))
        .route("/automations/{id}/steps/{sid}/move-up", axum::routing::post(post_move_step_up_handler))
        .route("/automations/{id}/steps/{sid}/move-down", axum::routing::post(post_move_step_down_handler))
        .route("/automations/{id}/steps/{sid}/delete", axum::routing::post(post_delete_step_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("auto_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;
    let csrf_token = app::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        &config.session_secret,
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
    let location = res.headers().get(header::LOCATION).unwrap().to_str().unwrap();
    assert!(location.starts_with("/automations/"));
    let automation_id: i64 = location.trim_start_matches("/automations/").parse().unwrap();

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
    let step1_body = format!("key_combo=F5&label=Refresh&post_delay_seconds=1.0&csrf_token={}", csrf_token);
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
    let step2_body = format!("key_combo=enter&label=Confirm&post_delay_seconds=0.5&csrf_token={}", csrf_token);
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
    let steps = sqlx::query("SELECT id, position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC")
        .bind(automation_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(steps.len(), 2);
    let step1_id: i64 = sqlx::Row::get(&steps[0], "id");
    let step2_id: i64 = sqlx::Row::get(&steps[1], "id");

    // 4. Move step 2 UP
    let move_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}/move-up", automation_id, step2_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(move_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify step 2 is now first
    let steps_after_move = sqlx::query("SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC")
        .bind(automation_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    let first_id: i64 = sqlx::Row::get(&steps_after_move[0], "id");
    assert_eq!(first_id, step2_id);

    // 5. Delete step 1
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}/delete", automation_id, step1_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(move_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let remaining_steps = sqlx::query("SELECT id FROM automation_steps WHERE automation_id = $1")
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

    let auto_check = sqlx::query("SELECT id FROM automations WHERE id = $1")
        .bind(automation_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(auto_check.is_none());
}
