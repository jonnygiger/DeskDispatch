#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::{AppState, config::{Config, SecretString}, routes::*};

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
use sqlx::PgPool;
use tower::ServiceExt;

struct MediaCleanupGuard {
    pool: PgPool,
    user_ids: Vec<i64>,
    automation_ids: Vec<i64>,
    bitmap_ids: Vec<i64>,
}

impl Drop for MediaCleanupGuard {
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
            if !bitmap_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM bitmaps WHERE id = ANY($1)")
                    .bind(&bitmap_ids)
                    .execute(&pool)
                    .await;
            }
            if !automation_ids.is_empty() {
                let _ = sqlx::query(
                    "DELETE FROM step_screenshots WHERE step_id IN (SELECT id FROM automation_steps WHERE automation_id = ANY($1))",
                )
                .bind(&automation_ids)
                .execute(&pool)
                .await;
                let _ = sqlx::query("DELETE FROM automation_steps WHERE automation_id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM automations WHERE id = ANY($1)")
                    .bind(&automation_ids)
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
async fn test_media_screenshot_redirects() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_media_screenshot_redirects");
        return;
    };

    let config = create_test_config();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        Some("http://localhost:9000"),
    );

    let (tx, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool.clone(),
        storage,
        config: config.clone(),
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tx,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = deskdispatch::build_router(state);

    let (user_id, session) = create_test_user(
        &pool,
        &format!("media_user_{}", uuid::Uuid::new_v4().simple()),
        "viewer",
    )
    .await;

    let mut cleanup_guard = MediaCleanupGuard {
        pool: pool.clone(),
        user_ids: vec![user_id],
        automation_ids: Vec::new(),
        bitmap_ids: Vec::new(),
    };

    // Insert dummy automation, step, and screenshot
    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Media Test', 'Desc', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.automation_ids.push(auto_id);

    let step_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 10.0, 'key_press') RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

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

    assert!(
        location.contains(&obj_key),
        "Location should contain object key"
    );
    assert!(
        location.contains("X-Amz-Expires="),
        "Location should be presigned S3 URL"
    );

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
    let bitmap_id: i64 = sqlx::query_scalar(
        "INSERT INTO bitmaps (automation_id, name, object_storage_key, width, height, created_by) VALUES ($1, 'Ref Image', $2, 100, 100, $3) RETURNING id",
    )
    .bind(auto_id)
    .bind(&bitmap_key)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.bitmap_ids.push(bitmap_id);

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

    assert!(
        location_bm.contains(&bitmap_key),
        "Location should contain bitmap key"
    );
    assert!(
        location_bm.contains("X-Amz-Expires="),
        "Location should be presigned S3 URL"
    );

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

#[tokio::test]
async fn test_dev_magnifier_verify_route() {
    let app = Router::new().route(
        "/dev/magnifier-verify",
        axum::routing::get(dev_magnifier_verify_handler),
    );

    // 1. Default request
    let req = Request::builder()
        .method("GET")
        .uri("/dev/magnifier-verify")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body_str.contains("Magnifier Component Verification"));
    assert!(body_str.contains("shot--normal"));
    assert!(body_str.contains("shot--zoom400"));
    assert!(body_str.contains("shot--grid"));
    assert!(body_str.contains("marker"));

    // 2. Query params request
    let req_params = Request::builder()
        .method("GET")
        .uri("/dev/magnifier-verify?url=http://example.com/test.png&native_w=1920&native_h=1080&target_x=500&target_y=400")
        .body(Body::empty())
        .unwrap();

    let res_params = app.clone().oneshot(req_params).await.unwrap();
    assert_eq!(res_params.status(), StatusCode::OK);

    let body_bytes_params = axum::body::to_bytes(res_params.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str_params = String::from_utf8(body_bytes_params.to_vec()).unwrap();

    assert!(body_str_params.contains("1920x1080"));
    assert!(body_str_params.contains("(500, 400)"));
    assert!(body_str_params.contains("http://example.com/test.png"));
}
