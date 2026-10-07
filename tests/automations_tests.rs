use app::{
    config::Config,
    routes::*,
    AppState,
};
use secrecy::ExposeSecret;
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
        .route("/automations/{id}/steps/new/mouse_click", axum::routing::get(get_new_mouse_click_step_handler))
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

#[tokio::test]
async fn test_mouse_click_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_mouse_click_step_crud_and_validation");
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
        .route("/automations/{id}", axum::routing::get(get_automation_detail_handler))
        .route("/automations/{id}/steps/new/mouse_click", axum::routing::get(get_new_mouse_click_step_handler))
        .route("/automations/{id}/steps", axum::routing::post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", axum::routing::get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", axum::routing::post(post_edit_step_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("mc_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;
    let csrf_token = app::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!("name=Mouse+Click+Automation&description=Test&csrf_token={}", csrf_token);
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
    let automation_id: i64 = location.trim_start_matches("/automations/").parse().unwrap();

    // 2. GET new mouse_click step page with query params
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/new/mouse_click?x=824&y=391", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. POST invalid mouse_click (missing X coordinate)
    let invalid_body = format!("step_type=mouse_click&x_mode=fixed&y_mode=fixed&y=100&csrf_token={}", csrf_token);
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
    let step_row = sqlx::query("SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let step_id: i64 = sqlx::Row::get(&step_row, "id");
    let step_type: String = sqlx::Row::get(&step_row, "step_type");
    let label: Option<String> = sqlx::Row::get(&step_row, "label");
    let post_delay_ms: i32 = sqlx::Row::get(&step_row, "post_delay_ms");

    assert_eq!(step_type, "mouse_click");
    assert_eq!(label.as_deref(), Some("Click Submit"));
    assert_eq!(post_delay_ms, 500);

    let click_row = sqlx::query("SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
        .bind(step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let x: Option<i32> = sqlx::Row::get(&click_row, "x");
    let y: Option<i32> = sqlx::Row::get(&click_row, "y");
    let button: String = sqlx::Row::get(&click_row, "button");
    let click_type: String = sqlx::Row::get(&click_row, "click_type");

    assert_eq!(x, Some(824));
    assert_eq!(y, Some(391));
    assert_eq!(button, "left");
    assert_eq!(click_type, "single");

    // 5. GET edit mouse_click page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/{}/edit", automation_id, step_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Create variable and update step to variable binding
    let var_row = sqlx::query("INSERT INTO automation_variables (automation_id, name, var_type) VALUES ($1, 'target_x', 'int') RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let var_id: i64 = sqlx::Row::get(&var_row, "id");

    let edit_body = format!(
        "step_type=mouse_click&label=Click+Variable&x_mode=variable&x_variable_id={}&y_mode=fixed&y=400&button=right&click_type=double&post_delay_seconds=1.0&csrf_token={}",
        var_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_click_row = sqlx::query("SELECT x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks WHERE step_id = $1")
        .bind(step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let updated_x: Option<i32> = sqlx::Row::get(&updated_click_row, "x");
    let updated_y: Option<i32> = sqlx::Row::get(&updated_click_row, "y");
    let updated_x_var: Option<i64> = sqlx::Row::get(&updated_click_row, "x_variable_id");
    let updated_button: String = sqlx::Row::get(&updated_click_row, "button");
    let updated_click_type: String = sqlx::Row::get(&updated_click_row, "click_type");

    assert_eq!(updated_x, None);
    assert_eq!(updated_x_var, Some(var_id));
    assert_eq!(updated_y, Some(400));
    assert_eq!(updated_button, "right");
    assert_eq!(updated_click_type, "double");

    // 7. Verify plain-language string rendering in detail page for mouse_click with variable
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Click («target_x», 400) [right, double]"));
}

#[tokio::test]
async fn test_find_pixel_rgb_step_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_find_pixel_rgb_step_crud_and_validation");
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
        .route("/automations/{id}", axum::routing::get(get_automation_detail_handler))
        .route("/automations/{id}/steps/new/find_pixel_rgb", axum::routing::get(get_new_find_pixel_rgb_step_handler))
        .route("/automations/{id}/steps", axum::routing::post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", axum::routing::get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", axum::routing::post(post_edit_step_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("fp_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;
    let csrf_token = app::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!("name=Pixel+RGB+Automation&description=Test&csrf_token={}", csrf_token);
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
    let automation_id: i64 = location.trim_start_matches("/automations/").parse().unwrap();

    // 2. GET new find_pixel_rgb step page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/new/find_pixel_rgb?x=100&y=200", automation_id))
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
    let step_row = sqlx::query("SELECT id, step_type, label, post_delay_ms FROM automation_steps WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let step_id: i64 = sqlx::Row::get(&step_row, "id");
    let step_type: String = sqlx::Row::get(&step_row, "step_type");
    let label: Option<String> = sqlx::Row::get(&step_row, "label");
    let post_delay_ms: i32 = sqlx::Row::get(&step_row, "post_delay_ms");

    assert_eq!(step_type, "find_pixel_rgb");
    assert_eq!(label.as_deref(), Some("Sample Pixel"));
    assert_eq!(post_delay_ms, 500);

    let fp_row = sqlx::query("SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1")
        .bind(step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let x: i32 = sqlx::Row::get(&fp_row, "x");
    let y: i32 = sqlx::Row::get(&fp_row, "y");
    let output_var_id: Option<i64> = sqlx::Row::get(&fp_row, "output_variable_id");

    assert_eq!(x, 100);
    assert_eq!(y, 200);
    assert_eq!(output_var_id, None);

    // 5. GET edit page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/{}/edit", automation_id, step_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Create output variable and update step
    let var_row = sqlx::query("INSERT INTO automation_variables (automation_id, name, var_type) VALUES ($1, 'bg_color', 'color') RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let var_id: i64 = sqlx::Row::get(&var_row, "id");

    let edit_body = format!(
        "step_type=find_pixel_rgb&label=Updated+Pixel&x=150&y=250&output_variable_id={}&post_delay_seconds=1.0&csrf_token={}",
        var_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/steps/{}", automation_id, step_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated values in DB
    let updated_fp_row = sqlx::query("SELECT x, y, output_variable_id FROM step_find_pixel_rgb WHERE step_id = $1")
        .bind(step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let updated_x: i32 = sqlx::Row::get(&updated_fp_row, "x");
    let updated_y: i32 = sqlx::Row::get(&updated_fp_row, "y");
    let updated_output_var_id: Option<i64> = sqlx::Row::get(&updated_fp_row, "output_variable_id");

    assert_eq!(updated_x, 150);
    assert_eq!(updated_y, 250);
    assert_eq!(updated_output_var_id, Some(var_id));

    // 7. Verify plain-language string rendering in detail page
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Read pixel at (150, 250) → store as «bg_color»"));
}

#[tokio::test]
async fn test_step_type_picker_interface() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_step_type_picker_interface");
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
        .route("/automations/{id}/steps/new", axum::routing::get(get_step_type_picker_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("picker_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;

    let req = Request::builder()
        .method("GET")
        .uri("/automations/42/steps/new")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body_str.contains("Select Step Type"));
    assert!(body_str.contains("/automations/42/steps/new/key_press"));
    assert!(body_str.contains("/automations/42/steps/new/mouse_click"));
    assert!(body_str.contains("/automations/42/steps/new/find_pixel_rgb"));
    assert!(body_str.contains("Key Press"));
    assert!(body_str.contains("Mouse Click"));
    assert!(body_str.contains("Find Pixel RGB"));
    assert!(body_str.contains("Find Bitmap"));
    assert!(body_str.contains("Branch"));
}

#[tokio::test]
async fn test_branch_step_target_selectors_and_foreign_keys() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_branch_step_target_selectors_and_foreign_keys");
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
        .route("/automations/{id}", axum::routing::get(get_automation_detail_handler))
        .route("/automations/{id}/steps/new/branch", axum::routing::get(get_new_branch_step_handler))
        .route("/automations/{id}/steps", axum::routing::post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", axum::routing::get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", axum::routing::post(post_edit_step_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("branch_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;
    let csrf_token = app::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!("name=Branch+Target+Automation&description=Test&csrf_token={}", csrf_token);
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
    let automation_id: i64 = location.trim_start_matches("/automations/").parse().unwrap();

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
    let steps = sqlx::query("SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC")
        .bind(automation_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(steps.len(), 2);
    let step1_id: i64 = sqlx::Row::get(&steps[0], "id");
    let step2_id: i64 = sqlx::Row::get(&steps[1], "id");

    // 3. GET /automations/{id}/steps/new/branch and check target step dropdown options
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/new/branch", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
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
    let steps_all = sqlx::query("SELECT id, step_type FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC")
        .bind(automation_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(steps_all.len(), 3);
    let branch_step_id: i64 = sqlx::Row::get(&steps_all[2], "id");

    // Verify foreign keys in step_branches
    let branch_row = sqlx::query("SELECT condition_type, automation_id, on_match_step_id, on_no_match_step_id FROM step_branches WHERE step_id = $1")
        .bind(branch_step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let cond_type: String = sqlx::Row::get(&branch_row, "condition_type");
    let branch_auto_id: i64 = sqlx::Row::get(&branch_row, "automation_id");
    let match_target: Option<i64> = sqlx::Row::get(&branch_row, "on_match_step_id");
    let no_match_target: Option<i64> = sqlx::Row::get(&branch_row, "on_no_match_step_id");

    assert_eq!(cond_type, "pixel_rgb");
    assert_eq!(branch_auto_id, automation_id);
    assert_eq!(match_target, Some(step1_id));
    assert_eq!(no_match_target, Some(step2_id));

    // 5. GET edit branch step page and check selected targets
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/steps/{}/edit", automation_id, branch_step_id))
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
        .uri(format!("/automations/{}/steps/{}", automation_id, branch_step_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(branch_edit_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Verify updated foreign keys in step_branches
    let updated_branch_row = sqlx::query("SELECT on_match_step_id, on_no_match_step_id FROM step_branches WHERE step_id = $1")
        .bind(branch_step_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let updated_match_target: Option<i64> = sqlx::Row::get(&updated_branch_row, "on_match_step_id");
    let updated_no_match_target: Option<i64> = sqlx::Row::get(&updated_branch_row, "on_no_match_step_id");

    assert_eq!(updated_match_target, Some(step2_id));
    assert_eq!(updated_no_match_target, Some(step1_id));

    // 7. Verify plain-language branch step description rendering in automation detail view
    let req = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("BRANCH: if pixel at (100, 200) ≈ RGB(10,20,30) ±5 → go to Step 2 (Step Beta), else → go to Step 1 (Step Alpha)"));
}

#[tokio::test]
async fn test_automation_parameters_crud_and_validation() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_automation_parameters_crud_and_validation");
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
        .route("/automations/{id}/parameters", axum::routing::get(get_automation_parameters_handler).post(post_create_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}", axum::routing::post(post_update_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}/delete", axum::routing::post(post_delete_automation_parameter_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_admin_id, admin_session) = create_test_user(&pool, &format!("param_admin_{}", uuid::Uuid::new_v4().simple()), "admin").await;
    let csrf_token = app::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Create automation
    let create_body = format!("name=Param+Test+Automation&description=Test&csrf_token={}", csrf_token);
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
    let automation_id: i64 = location.trim_start_matches("/automations/").parse().unwrap();

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
    let param_row = sqlx::query("SELECT id, name, param_type, default_value, description FROM automation_parameters WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let param_id: i64 = sqlx::Row::get(&param_row, "id");
    let name: String = sqlx::Row::get(&param_row, "name");
    let param_type: String = sqlx::Row::get(&param_row, "param_type");
    let default_val: String = sqlx::Row::get(&param_row, "default_value");

    assert_eq!(name, "click_tolerance");
    assert_eq!(param_type, "int");
    assert_eq!(default_val, "10");

    // 5. POST update parameter
    let update_param_body = format!(
        "name=click_tolerance&param_type=int&default_value=15&description=Updated+Tolerance&csrf_token={}",
        csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/parameters/{}", automation_id, param_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(update_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let updated_row = sqlx::query("SELECT default_value, description FROM automation_parameters WHERE id = $1")
        .bind(param_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let updated_val: String = sqlx::Row::get(&updated_row, "default_value");
    let updated_desc: String = sqlx::Row::get(&updated_row, "description");
    assert_eq!(updated_val, "15");
    assert_eq!(updated_desc, "Updated Tolerance");

    // 6. POST delete parameter
    let delete_param_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/parameters/{}/delete", automation_id, param_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(delete_param_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM automation_parameters WHERE automation_id = $1")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
