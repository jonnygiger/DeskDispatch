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
async fn test_media_screenshot_redirects() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_media_screenshot_redirects");
        return;
    };

    let config = Config::from_env().unwrap();
    let credentials = aws_sdk_s3::config::Credentials::new("key", "secret", None, None, "static");
    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .endpoint_url("http://localhost:9000")
        .force_path_style(true)
        .build();
    let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

    let state = AppState {
        db: pool.clone(),
        s3_client,
        config: config.clone(),
        rate_limiter: app::auth::LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/media/screenshots/{id}", axum::routing::get(get_media_screenshot_handler))
        .route("/media/bitmaps/{id}", axum::routing::get(get_media_bitmap_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (user_id, session) = create_test_user(&pool, &format!("media_user_{}", uuid::Uuid::new_v4().simple()), "viewer").await;

    // Insert dummy automation, step, and screenshot
    let auto_row = sqlx::query(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Media Test', 'Desc', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let auto_id: i64 = sqlx::Row::get(&auto_row, "id");

    let step_row = sqlx::query(
        "INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 10.0, 'key_press') RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let step_id: i64 = sqlx::Row::get(&step_row, "id");

    let obj_key = format!("screenshots/step_{}.png", step_id);
    sqlx::query(
        "INSERT INTO step_screenshots (step_id, object_storage_key, width, height) VALUES ($1, $2, 1920, 1080)",
    )
    .bind(step_id)
    .bind(&obj_key)
    .execute(&pool)
    .await
    .unwrap();

    // 1. GET existing screenshot -> Expect 302 FOUND with Location header
    let req = Request::builder()
        .method("GET")
        .uri(format!("/media/screenshots/{}", step_id))
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);

    let location = res
        .headers()
        .get(header::LOCATION)
        .expect("Location header should be set")
        .to_str()
        .unwrap();

    assert!(location.contains(&obj_key), "Location should contain object key");
    assert!(location.contains("X-Amz-Expires="), "Location should be presigned S3 URL");

    // 2. GET non-existent screenshot -> Expect 404 NOT_FOUND
    let req_nf = Request::builder()
        .method("GET")
        .uri("/media/screenshots/99999999")
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_nf = app.clone().oneshot(req_nf).await.unwrap();
    assert_eq!(res_nf.status(), StatusCode::NOT_FOUND);

    // Insert dummy bitmap
    let bitmap_key = format!("bitmaps/ref_{}.png", uuid::Uuid::new_v4().simple());
    let bitmap_row = sqlx::query(
        "INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by) VALUES ($1, 'Ref Image', $2, 100, 100, $3) RETURNING id",
    )
    .bind(auto_id)
    .bind(&bitmap_key)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let bitmap_id: i64 = sqlx::Row::get(&bitmap_row, "id");

    // 3. GET existing bitmap -> Expect 302 FOUND with Location header
    let req_bm = Request::builder()
        .method("GET")
        .uri(format!("/media/bitmaps/{}", bitmap_id))
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_bm = app.clone().oneshot(req_bm).await.unwrap();
    assert_eq!(res_bm.status(), StatusCode::FOUND);

    let location_bm = res_bm
        .headers()
        .get(header::LOCATION)
        .expect("Location header should be set")
        .to_str()
        .unwrap();

    assert!(location_bm.contains(&bitmap_key), "Location should contain bitmap key");
    assert!(location_bm.contains("X-Amz-Expires="), "Location should be presigned S3 URL");

    // 4. GET non-existent bitmap -> Expect 404 NOT_FOUND
    let req_bm_nf = Request::builder()
        .method("GET")
        .uri("/media/bitmaps/99999999")
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_bm_nf = app.clone().oneshot(req_bm_nf).await.unwrap();
    assert_eq!(res_bm_nf.status(), StatusCode::NOT_FOUND);
}
