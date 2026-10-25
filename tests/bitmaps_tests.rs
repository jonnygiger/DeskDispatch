#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::{AppState, config::Config, routes::*};
use sqlx::PgPool;
use tower::ServiceExt;

#[derive(sqlx::FromRow)]
struct BitmapTestRow {
    id: i64,
    name: String,
    #[allow(dead_code)]
    object_storage_key: String,
    width: i32,
    height: i32,
}

#[derive(sqlx::FromRow)]
struct ScopedBitmapTestRow {
    id: i64,
    automation_id: Option<i64>,
    width: i32,
    height: i32,
}

#[derive(sqlx::FromRow)]
struct BitmapCropRow {
    id: i64,
    width: i32,
    height: i32,
    created_by: i64,
}

#[derive(sqlx::FromRow)]
struct AuditLogRow {
    action: String,
}

struct BitmapsCleanupGuard {
    pool: PgPool,
    user_ids: Vec<i64>,
    automation_ids: Vec<i64>,
    bitmap_ids: Vec<i64>,
}

impl Drop for BitmapsCleanupGuard {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let user_ids = self.user_ids.clone();
        let automation_ids = self.automation_ids.clone();
        let bitmap_ids = self.bitmap_ids.clone();

        let cleanup = async move {
            if !bitmap_ids.is_empty() {
                let _ = sqlx::query(
                    "DELETE FROM audit_log WHERE entity_type = 'bitmap' AND entity_id = ANY($1)",
                )
                .bind(&bitmap_ids)
                .execute(&pool)
                .await;
                let _ = sqlx::query("DELETE FROM bitmaps WHERE id = ANY($1)")
                    .bind(&bitmap_ids)
                    .execute(&pool)
                    .await;
            }
            if !automation_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM bitmaps WHERE automation_id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM automations WHERE id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
            }
            if !user_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM audit_log WHERE user_id = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM bitmaps WHERE created_by = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM automations WHERE created_by = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
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
async fn test_bitmaps_list_routes() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_bitmaps_list_routes");
        return;
    };

    let mut cleanup_guard = BitmapsCleanupGuard {
        pool: pool.clone(),
        user_ids: Vec::new(),
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    let config = Config::from_env().unwrap();
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
        .route("/bitmaps", axum::routing::get(get_bitmaps_handler))
        .route(
            "/automations/{id}/bitmaps",
            axum::routing::get(get_automation_bitmaps_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

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
    let (user_id, session) = create_test_user(
        &pool,
        &format!("bitmap_user_{}", uuid::Uuid::new_v4().simple()),
        "viewer",
    )
    .await;
    cleanup_guard.user_ids.push(user_id);

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
    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Bitmap Test Auto', 'Desc', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.automation_ids.push(auto_id);

    let bitmap_key = format!("bitmaps/test_{}.png", uuid::Uuid::new_v4().simple());
    let bitmap_id: i64 = sqlx::query_scalar(
        "INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by) VALUES ($1, 'OK Button Bitmap', $2, 200, 80, $3) RETURNING id",
    )
    .bind(auto_id)
    .bind(&bitmap_key)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.bitmap_ids.push(bitmap_id);

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

    let mut cleanup_guard = BitmapsCleanupGuard {
        pool: pool.clone(),
        user_ids: Vec::new(),
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    let config = Config::from_env().unwrap();
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
            "/bitmaps",
            axum::routing::get(get_bitmaps_handler).post(post_bitmaps_handler),
        )
        .route(
            "/automations/{id}/bitmaps",
            axum::routing::get(get_automation_bitmaps_handler)
                .post(post_automation_bitmaps_handler),
        )
        .route(
            "/bitmaps/commit",
            axum::routing::get(get_bitmap_commit_handler).post(bitmaps::post_bitmap_commit_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state.clone());

    let (viewer_id, viewer_session) = create_test_user(
        &pool,
        &format!("viewer_upload_{}", uuid::Uuid::new_v4().simple()),
        "viewer",
    )
    .await;
    cleanup_guard.user_ids.push(viewer_id);

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("admin_upload_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;
    cleanup_guard.user_ids.push(admin_id);

    // Extract CSRF token for admin session
    let session_uuid = uuid::Uuid::parse_str(&admin_session).unwrap();
    let admin_csrf = deskdispatch::auth::generate_csrf_token(
        session_uuid,
        config.session_secret.expose_secret(),
    );

    // Extract CSRF token for viewer session
    let viewer_session_uuid = uuid::Uuid::parse_str(&viewer_session).unwrap();
    let viewer_csrf = deskdispatch::auth::generate_csrf_token(
        viewer_session_uuid,
        config.session_secret.expose_secret(),
    );

    // 1. Viewer attempt to POST /bitmaps -> 403 Forbidden
    let req_viewer = Request::builder()
        .method("POST")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", viewer_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf_token={}&name=ForbiddenBitmap",
            viewer_csrf
        )))
        .unwrap();

    let res_viewer = app.clone().oneshot(req_viewer).await.unwrap();
    assert_eq!(res_viewer.status(), StatusCode::FORBIDDEN);

    // 2. Admin POST /bitmaps -> 200 OK with presigned POST upload form
    let req_admin = Request::builder()
        .method("POST")
        .uri("/bitmaps")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf_token={}&name=New%20Admin%20Bitmap",
            admin_csrf
        )))
        .unwrap();

    let res_admin = app.clone().oneshot(req_admin).await.unwrap();
    assert_eq!(res_admin.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res_admin.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Upload Image for Reference Bitmap"));
    assert!(body_str.contains("New Admin Bitmap"));
    assert!(body_str.contains("http://localhost:9000/deskdispatch-bucket"));
    assert!(body_str.contains("success_action_redirect"));
    assert!(body_str.contains("/bitmaps/commit?name="));

    // Upload a valid 10x10 PNG object to S3 for commit testing
    let test_key = format!(
        "bitmaps/user_{}_{}.png",
        admin_id,
        uuid::Uuid::new_v4().simple()
    );
    let red_pixels = [255, 0, 0].repeat(10 * 10);
    let png_bytes = deskdispatch::routes::bitmaps::encode_png(10, 10, &red_pixels);

    let _ = state
        .storage_service()
        .put_object(&test_key, png_bytes, "image/png")
        .await;

    // 3. Admin GET /bitmaps/commit callback -> 200 OK rendering confirmation page (BitmapsConfirmTemplate)
    // S3/RustFS redirects to redirect_url by appending bucket, key, and etag parameters.
    let req_commit_get = Request::builder()
        .method("GET")
        .uri(format!(
            "/bitmaps/commit?name=Committed%20Bitmap&bucket=deskdispatch-bucket&key={}&etag=%2212345%22",
            test_key
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();

    let res_commit_get = app.clone().oneshot(req_commit_get).await.unwrap();
    assert_eq!(res_commit_get.status(), StatusCode::OK);
    let get_body_bytes = axum::body::to_bytes(res_commit_get.into_body(), usize::MAX)
        .await
        .unwrap();
    let get_body_str = String::from_utf8(get_body_bytes.to_vec()).unwrap();
    assert!(get_body_str.contains("Step 3: Confirm Reference Bitmap Registration"));
    assert!(get_body_str.contains("10 × 10 px"));
    assert!(get_body_str.contains("/bitmaps/commit"));

    // 4. Admin POST /bitmaps/commit with CSRF token -> 303 Redirect to /bitmaps
    let req_commit_post = Request::builder()
        .method("POST")
        .uri("/bitmaps/commit")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf_token={}&key={}&name=Committed%20Bitmap",
            admin_csrf, test_key
        )))
        .unwrap();

    let res_commit_post = app.clone().oneshot(req_commit_post).await.unwrap();
    assert_eq!(res_commit_post.status(), StatusCode::SEE_OTHER);
    let location = res_commit_post
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/bitmaps");

    // Verify row inserted in database with correct dimensions (10x10)
    let inserted_row = sqlx::query_as::<_, BitmapTestRow>(
        "SELECT id, name, object_storage_key, width, height FROM bitmaps WHERE object_storage_key = $1",
    )
    .bind(&test_key)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(inserted_row.is_some());
    let row = inserted_row.unwrap();
    cleanup_guard.bitmap_ids.push(row.id);

    assert_eq!(row.name, "Committed Bitmap");
    assert_eq!(row.width, 10);
    assert_eq!(row.height, 10);

    // 5. Idempotency test: Re-submitting POST /bitmaps/commit returns redirect without inserting duplicate
    let req_commit_post_dup = Request::builder()
        .method("POST")
        .uri("/bitmaps/commit")
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf_token={}&key={}&name=Committed%20Bitmap",
            admin_csrf, test_key
        )))
        .unwrap();

    let res_commit_post_dup = app.clone().oneshot(req_commit_post_dup).await.unwrap();
    assert_eq!(res_commit_post_dup.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_bitmap_deletion_flow() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_bitmap_deletion_flow");
        return;
    };

    let mut cleanup_guard = BitmapsCleanupGuard {
        pool: pool.clone(),
        user_ids: Vec::new(),
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    let config = Config::from_env().unwrap();
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
            "/bitmaps/{id}/delete",
            axum::routing::post(post_delete_bitmap_handler),
        )
        .route(
            "/automations/{id}/bitmaps/{bid}/delete",
            axum::routing::post(post_automation_delete_bitmap_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state);

    let (viewer_id, viewer_session) = create_test_user(
        &pool,
        &format!("viewer_del_{}", uuid::Uuid::new_v4().simple()),
        "viewer",
    )
    .await;
    cleanup_guard.user_ids.push(viewer_id);

    let (editor_id, editor_session) = create_test_user(
        &pool,
        &format!("editor_del_{}", uuid::Uuid::new_v4().simple()),
        "editor",
    )
    .await;
    cleanup_guard.user_ids.push(editor_id);

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("admin_del_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;
    cleanup_guard.user_ids.push(admin_id);

    let viewer_csrf = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&viewer_session).unwrap(),
        config.session_secret.expose_secret(),
    );
    let editor_csrf = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&editor_session).unwrap(),
        config.session_secret.expose_secret(),
    );
    let admin_csrf = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // Insert a test bitmap record
    let key1 = format!("bitmaps/del_test_1_{}.png", uuid::Uuid::new_v4().simple());
    let bm_id_1: i64 = sqlx::query_scalar(
        "INSERT INTO bitmaps (name, object_storage_key, width, height, created_by) VALUES ('Bitmap To Delete 1', $1, 100, 100, $2) RETURNING id",
    )
    .bind(&key1)
    .bind(editor_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.bitmap_ids.push(bm_id_1);

    // 1. Viewer attempt to delete -> 403 Forbidden
    let req_viewer = Request::builder()
        .method("POST")
        .uri(format!("/bitmaps/{}/delete", bm_id_1))
        .header(header::COOKIE, format!("session_id={}", viewer_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={}", viewer_csrf)))
        .unwrap();

    let res_viewer = app.clone().oneshot(req_viewer).await.unwrap();
    assert_eq!(res_viewer.status(), StatusCode::FORBIDDEN);

    // 2. Editor attempt with bad CSRF token -> 400 Bad Request
    let req_bad_csrf = Request::builder()
        .method("POST")
        .uri(format!("/bitmaps/{}/delete", bm_id_1))
        .header(header::COOKIE, format!("session_id={}", editor_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("csrf_token=invalid_token"))
        .unwrap();

    let res_bad_csrf = app.clone().oneshot(req_bad_csrf).await.unwrap();
    assert_eq!(res_bad_csrf.status(), StatusCode::BAD_REQUEST);

    // 3. Editor attempt with valid CSRF token -> 303 Redirect to /bitmaps
    let req_editor = Request::builder()
        .method("POST")
        .uri(format!("/bitmaps/{}/delete", bm_id_1))
        .header(header::COOKIE, format!("session_id={}", editor_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={}", editor_csrf)))
        .unwrap();

    let res_editor = app.clone().oneshot(req_editor).await.unwrap();
    assert_eq!(res_editor.status(), StatusCode::SEE_OTHER);
    let loc_editor = res_editor
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(loc_editor, "/bitmaps");

    // Verify record deleted from database
    let check_row1: Option<i64> = sqlx::query_scalar("SELECT id FROM bitmaps WHERE id = $1")
        .bind(bm_id_1)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(check_row1.is_none());

    // Verify audit log entry
    let audit_row1 = sqlx::query_as::<_, AuditLogRow>(
        "SELECT action FROM audit_log WHERE entity_type = 'bitmap' AND entity_id = $1",
    )
    .bind(bm_id_1)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(audit_row1.is_some());
    assert_eq!(audit_row1.unwrap().action, "delete_bitmap");

    // Insert an automation and scoped bitmap record
    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Auto For Delete Test', '', 'draft', $1) RETURNING id",
    )
    .bind(admin_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.automation_ids.push(auto_id);

    let key2 = format!("bitmaps/del_test_2_{}.png", uuid::Uuid::new_v4().simple());
    let bm_id_2: i64 = sqlx::query_scalar(
        "INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by) VALUES ($1, 'Scoped Bitmap To Delete', $2, 50, 50, $3) RETURNING id",
    )
    .bind(auto_id)
    .bind(&key2)
    .bind(admin_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.bitmap_ids.push(bm_id_2);

    // 4. Admin delete scoped bitmap via /automations/{id}/bitmaps/{bid}/delete
    let req_admin_scoped = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/bitmaps/{}/delete",
            auto_id, bm_id_2
        ))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={}", admin_csrf)))
        .unwrap();

    let res_admin_scoped = app.clone().oneshot(req_admin_scoped).await.unwrap();
    assert_eq!(res_admin_scoped.status(), StatusCode::SEE_OTHER);
    let loc_admin_scoped = res_admin_scoped
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        loc_admin_scoped,
        format!("/automations/{}/bitmaps", auto_id)
    );

    // Verify scoped record deleted from database
    let check_row2: Option<i64> = sqlx::query_scalar("SELECT id FROM bitmaps WHERE id = $1")
        .bind(bm_id_2)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(check_row2.is_none());
}

#[tokio::test]
async fn test_region_picker_top_left_flow() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_region_picker_top_left_flow");
        return;
    };

    let mut cleanup_guard = BitmapsCleanupGuard {
        pool: pool.clone(),
        user_ids: Vec::new(),
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    let config = Config::from_env().unwrap();
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
            "/bitmaps/pick-region",
            axum::routing::get(get_pick_region_handler).post(post_pick_region_top_left_handler),
        )
        .route(
            "/automations/{id}/bitmaps/pick-region",
            axum::routing::get(get_automation_pick_region_handler)
                .post(post_automation_pick_region_top_left_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state);

    let (user_id, session) = create_test_user(
        &pool,
        &format!("region_user_{}", uuid::Uuid::new_v4().simple()),
        "editor",
    )
    .await;
    cleanup_guard.user_ids.push(user_id);

    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. GET /bitmaps/pick-region
    let req_get = Request::builder()
        .method("GET")
        .uri("/bitmaps/pick-region?image_url=/static/sample.png&width=1200&height=680")
        .header(header::COOKIE, format!("session_id={}", session))
        .body(Body::empty())
        .unwrap();

    let res_get = app.clone().oneshot(req_get).await.unwrap();
    assert_eq!(res_get.status(), StatusCode::OK);
    let body_bytes_get = axum::body::to_bytes(res_get.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_get = String::from_utf8(body_bytes_get.to_vec()).unwrap();
    assert!(body_str_get.contains("Step 1: Click Top-Left Corner"));
    assert!(body_str_get.contains("input type=\"image\""));

    // 2. POST /bitmaps/pick-region with image click coordinates (300, 170)
    let post_body = format!(
        "csrf_token={}&image_url=/static/sample.png&width=1200&height=680&click.x=300&click.y=170",
        csrf_token
    );

    let req_post = Request::builder()
        .method("POST")
        .uri("/bitmaps/pick-region")
        .header(header::COOKIE, format!("session_id={}", session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(post_body))
        .unwrap();

    let res_post = app.clone().oneshot(req_post).await.unwrap();
    assert_eq!(res_post.status(), StatusCode::OK);

    let body_bytes_post = axum::body::to_bytes(res_post.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_post = String::from_utf8(body_bytes_post.to_vec()).unwrap();
    assert!(body_str_post.contains("Coarse Top-Left Corner Selected"));
    assert!(body_str_post.contains("(600, 340)"));
}

#[tokio::test]
async fn test_region_picker_confirm_crop_flow() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_region_picker_confirm_crop_flow");
        return;
    };

    let mut cleanup_guard = BitmapsCleanupGuard {
        pool: pool.clone(),
        user_ids: Vec::new(),
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    let config = Config::from_env().unwrap();
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
            "/bitmaps/pick-region/bottom-right",
            axum::routing::post(post_pick_region_bottom_right_handler),
        )
        .route(
            "/bitmaps/pick-region/confirm",
            axum::routing::post(post_pick_region_confirm_handler),
        )
        .route(
            "/automations/{id}/bitmaps/pick-region/confirm",
            axum::routing::post(post_automation_pick_region_confirm_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state);

    let (viewer_id, viewer_session) = create_test_user(
        &pool,
        &format!("viewer_confirm_{}", uuid::Uuid::new_v4().simple()),
        "viewer",
    )
    .await;
    cleanup_guard.user_ids.push(viewer_id);

    let (editor_id, editor_session) = create_test_user(
        &pool,
        &format!("editor_confirm_{}", uuid::Uuid::new_v4().simple()),
        "editor",
    )
    .await;
    cleanup_guard.user_ids.push(editor_id);

    let viewer_csrf = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&viewer_session).unwrap(),
        config.session_secret.expose_secret(),
    );
    let editor_csrf = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&editor_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    // 1. Editor POST /bitmaps/pick-region/bottom-right -> Stage 3 HTML with Step 3 Confirm Crop Preview
    let br_post_body = format!(
        "csrf_token={}&image_url=/static/sample.png&width=1200&height=680&top_left_x=100&top_left_y=100&coarse_click.x=300&coarse_click.y=200",
        editor_csrf
    );

    let req_br = Request::builder()
        .method("POST")
        .uri("/bitmaps/pick-region/bottom-right")
        .header(header::COOKIE, format!("session_id={}", editor_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(br_post_body))
        .unwrap();

    let res_br = app.clone().oneshot(req_br).await.unwrap();
    assert_eq!(res_br.status(), StatusCode::OK);

    let body_bytes_br = axum::body::to_bytes(res_br.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_br = String::from_utf8(body_bytes_br.to_vec()).unwrap();
    assert!(body_str_br.contains("Step 3: Confirm Crop Preview"));
    assert!(body_str_br.contains("/bitmaps/pick-region/confirm"));
    assert!(body_str_br.contains("Confirm &amp; Save Bitmap"));

    // 2. Viewer POST /bitmaps/pick-region/confirm -> 403 Forbidden
    let viewer_confirm_body = format!(
        "csrf_token={}&name=ForbiddenCrop&image_url=/static/sample.png&width=1200&height=680&top_left_x=100&top_left_y=100&bottom_right_x=200&bottom_right_y=200",
        viewer_csrf
    );

    let req_viewer_confirm = Request::builder()
        .method("POST")
        .uri("/bitmaps/pick-region/confirm")
        .header(header::COOKIE, format!("session_id={}", viewer_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(viewer_confirm_body))
        .unwrap();

    let res_viewer_confirm = app.clone().oneshot(req_viewer_confirm).await.unwrap();
    assert_eq!(res_viewer_confirm.status(), StatusCode::FORBIDDEN);

    // 3. Editor POST /bitmaps/pick-region/confirm -> 303 Redirect to /bitmaps
    let editor_confirm_body = format!(
        "csrf_token={}&name=Crop%20Selection%20Bitmap&image_url=/static/sample.png&width=1200&height=680&top_left_x=100&top_left_y=100&bottom_right_x=199&bottom_right_y=149",
        editor_csrf
    );

    let req_editor_confirm = Request::builder()
        .method("POST")
        .uri("/bitmaps/pick-region/confirm")
        .header(header::COOKIE, format!("session_id={}", editor_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(editor_confirm_body))
        .unwrap();

    let res_editor_confirm = app.clone().oneshot(req_editor_confirm).await.unwrap();
    assert_eq!(res_editor_confirm.status(), StatusCode::SEE_OTHER);
    let location = res_editor_confirm
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/bitmaps");

    // Verify row inserted in database: width = 100, height = 50
    let crop_row = sqlx::query_as::<_, BitmapCropRow>(
        "SELECT id, width, height, created_by FROM bitmaps WHERE name = 'Crop Selection Bitmap'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(crop_row.is_some());
    let row = crop_row.unwrap();
    cleanup_guard.bitmap_ids.push(row.id);

    assert_eq!(row.width, 100);
    assert_eq!(row.height, 50);
    assert_eq!(row.created_by, editor_id);

    // Verify audit log entry
    let audit_row = sqlx::query_as::<_, AuditLogRow>(
        "SELECT action FROM audit_log WHERE entity_type = 'bitmap' AND entity_id = $1",
    )
    .bind(row.id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(audit_row.is_some());
    assert_eq!(audit_row.unwrap().action, "create_bitmap_crop");

    // 4. Scoped confirmation: POST /automations/{id}/bitmaps/pick-region/confirm
    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Scoped Confirm Auto', '', 'draft', $1) RETURNING id",
    )
    .bind(editor_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.automation_ids.push(auto_id);

    let scoped_confirm_body = format!(
        "csrf_token={}&name=Scoped%20Crop%20Bitmap&image_url=/static/sample.png&width=1200&height=680&top_left_x=50&top_left_y=50&bottom_right_x=89&bottom_right_y=89",
        editor_csrf
    );

    let req_scoped_confirm = Request::builder()
        .method("POST")
        .uri(format!(
            "/automations/{}/bitmaps/pick-region/confirm",
            auto_id
        ))
        .header(header::COOKIE, format!("session_id={}", editor_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(scoped_confirm_body))
        .unwrap();

    let res_scoped_confirm = app.clone().oneshot(req_scoped_confirm).await.unwrap();
    assert_eq!(res_scoped_confirm.status(), StatusCode::SEE_OTHER);
    let scoped_loc = res_scoped_confirm
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(scoped_loc, format!("/automations/{}/bitmaps", auto_id));

    let scoped_row = sqlx::query_as::<_, ScopedBitmapTestRow>(
        "SELECT id, automation_id, width, height FROM bitmaps WHERE name = 'Scoped Crop Bitmap'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(scoped_row.is_some());
    let s_row = scoped_row.unwrap();
    cleanup_guard.bitmap_ids.push(s_row.id);

    assert_eq!(s_row.automation_id, Some(auto_id));
    assert_eq!(s_row.width, 40);
    assert_eq!(s_row.height, 40);
}
