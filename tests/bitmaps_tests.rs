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
async fn test_bitmaps_list_routes() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_bitmaps_list_routes");
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
        .route("/bitmaps", axum::routing::get(get_bitmaps_handler))
        .route("/automations/{id}/bitmaps", axum::routing::get(get_automation_bitmaps_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    // 1. Unauthenticated request -> expect 303 or redirect to /login
    let req_unauth = Request::builder()
        .method("GET")
        .uri("/bitmaps")
        .body(Body::empty())
        .unwrap();

    let res_unauth = app.clone().oneshot(req_unauth).await.unwrap();
    assert_eq!(res_unauth.status(), StatusCode::SEE_OTHER);
    let location = res_unauth
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/login");

    // 2. Authenticated request empty list
    let (user_id, session) = create_test_user(&pool, &format!("bitmap_user_{}", uuid::Uuid::new_v4().simple()), "viewer").await;

    let req_auth = Request::builder()
        .method("GET")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_auth = app.clone().oneshot(req_auth).await.unwrap();
    assert_eq!(res_auth.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res_auth.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Reference Bitmap Library"));

    // 3. Insert test automation and bitmap records
    let auto_row = sqlx::query(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Bitmap Test Auto', 'Desc', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let auto_id: i64 = sqlx::Row::get(&auto_row, "id");

    let bitmap_key = format!("bitmaps/test_{}.png", uuid::Uuid::new_v4().simple());
    let bitmap_row = sqlx::query(
        "INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by) VALUES ($1, 'OK Button Bitmap', $2, 200, 80, $3) RETURNING id",
    )
    .bind(auto_id)
    .bind(&bitmap_key)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let bitmap_id: i64 = sqlx::Row::get(&bitmap_row, "id");

    // 4. GET /bitmaps with bitmap present
    let req_with_item = Request::builder()
        .method("GET")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_with_item = app.clone().oneshot(req_with_item).await.unwrap();
    assert_eq!(res_with_item.status(), StatusCode::OK);

    let body_bytes_item = axum::body::to_bytes(res_with_item.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_item = String::from_utf8(body_bytes_item.to_vec()).unwrap();
    assert!(body_str_item.contains("OK Button Bitmap"));
    assert!(body_str_item.contains("200 × 80 px"));
    assert!(body_str_item.contains(&format!("/media/bitmaps/{}", bitmap_id)));
    assert!(body_str_item.contains("shot--normal"));
    assert!(body_str_item.contains("shot--zoom400"));
    assert!(body_str_item.contains("shot--grid"));

    // 5. GET /automations/{id}/bitmaps
    let req_auto_bm = Request::builder()
        .method("GET")
        .uri(format!("/automations/{}/bitmaps", auto_id))
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_auto_bm = app.clone().oneshot(req_auto_bm).await.unwrap();
    assert_eq!(res_auto_bm.status(), StatusCode::OK);

    let body_bytes_auto_bm = axum::body::to_bytes(res_auto_bm.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_auto_bm = String::from_utf8(body_bytes_auto_bm.to_vec()).unwrap();
    assert!(body_str_auto_bm.contains("Reference Bitmaps: Bitmap Test Auto"));
    assert!(body_str_auto_bm.contains("OK Button Bitmap"));
}

#[tokio::test]
async fn test_bitmap_upload_flow() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_bitmap_upload_flow");
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
        .route("/bitmaps", axum::routing::get(get_bitmaps_handler).post(post_bitmaps_handler))
        .route("/automations/{id}/bitmaps", axum::routing::get(get_automation_bitmaps_handler).post(post_automation_bitmaps_handler))
        .route("/bitmaps/commit", axum::routing::get(get_bitmap_commit_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

    let (_viewer_id, viewer_session) = create_test_user(&pool, &format!("viewer_upload_{}", uuid::Uuid::new_v4().simple()), "viewer").await;
    let (_admin_id, admin_session) = create_test_user(&pool, &format!("admin_upload_{}", uuid::Uuid::new_v4().simple()), "admin").await;

    // Extract CSRF token for admin session
    let session_uuid = uuid::Uuid::parse_str(&admin_session).unwrap();
    let admin_csrf = app::auth::generate_csrf_token(session_uuid, &config.session_secret);

    // Extract CSRF token for viewer session
    let viewer_session_uuid = uuid::Uuid::parse_str(&viewer_session).unwrap();
    let viewer_csrf = app::auth::generate_csrf_token(viewer_session_uuid, &config.session_secret);

    // 1. Viewer attempt to POST /bitmaps -> 403 Forbidden
    let req_viewer = Request::builder()
        .method("POST")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", viewer_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={}&name=ForbiddenBitmap", viewer_csrf)))
        .unwrap();

    let res_viewer = app.clone().oneshot(req_viewer).await.unwrap();
    assert_eq!(res_viewer.status(), StatusCode::FORBIDDEN);

    // 2. Admin POST /bitmaps -> 200 OK with presigned POST upload form
    let req_admin = Request::builder()
        .method("POST")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={}&name=New%20Admin%20Bitmap", admin_csrf)))
        .unwrap();

    let res_admin = app.clone().oneshot(req_admin).await.unwrap();
    assert_eq!(res_admin.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res_admin.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Upload Image for Reference Bitmap"));
    assert!(body_str.contains("New Admin Bitmap"));
    assert!(body_str.contains("http://localhost:9000/deskdispatch-bucket"));
    assert!(body_str.contains("success_action_redirect"));
    assert!(body_str.contains("/bitmaps/commit?key="));

    // 3. Admin GET /bitmaps/commit callback
    let test_key = format!("bitmaps/commit_test_{}.png", uuid::Uuid::new_v4().simple());
    let req_commit = Request::builder()
        .method("GET")
        .uri(format!("/bitmaps/commit?key={}&name=Committed%20Bitmap", test_key))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();

    let res_commit = app.clone().oneshot(req_commit).await.unwrap();
    assert_eq!(res_commit.status(), StatusCode::SEE_OTHER);
    let location = res_commit.headers().get(header::LOCATION).unwrap().to_str().unwrap();
    assert_eq!(location, "/bitmaps");

    // Verify row inserted in database
    let inserted_row = sqlx::query("SELECT id, name, object_storage_key FROM bitmaps WHERE object_storage_key = $1")
        .bind(&test_key)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(inserted_row.is_some());
    let row = inserted_row.unwrap();
    let db_name: String = sqlx::Row::get(&row, "name");
    assert_eq!(db_name, "Committed Bitmap");
}
