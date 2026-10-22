#![allow(clippy::unwrap_used)]
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::AppState;
use deskdispatch::auth::LoginRateLimiter;
use deskdispatch::config::Config;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

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

struct DashboardCleanupGuard {
    pool: sqlx::PgPool,
    user_id: i64,
    worker1_id: i64,
    auto1_id: i64,
}

impl Drop for DashboardCleanupGuard {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let user_id = self.user_id;
        let worker1_id = self.worker1_id;
        let auto1_id = self.auto1_id;

        let cleanup = async move {
            let _ = sqlx::query("DELETE FROM task_runs WHERE automation_id = $1")
                .bind(auto1_id)
                .execute(&pool)
                .await;
            let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
                .bind(worker1_id)
                .execute(&pool)
                .await;
            let _ = sqlx::query("DELETE FROM automations WHERE created_by = $1")
                .bind(user_id)
                .execute(&pool)
                .await;
            let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                .bind(user_id)
                .execute(&pool)
                .await;
            let _ = sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(user_id)
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

#[tokio::test]
async fn test_dashboard_stats_tiles() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    // Seed test user with dummy hash (bypassing expensive password hashing)
    let username = format!("dashuser_{}", Uuid::new_v4().simple());

    let user_row = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, 'hash', 'Dashboard Tester', 'admin', true)
        RETURNING id
        "#,
    )
    .bind(&username)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user_id: i64 = user_row.get("id");

    // Insert automations and worker
    let auto1 = sqlx::query(
        "INSERT INTO automations (name, status, created_by) VALUES ('Auto 1', 'active', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let auto1_id: i64 = auto1.get("id");

    let _auto2 = sqlx::query(
        "INSERT INTO automations (name, status, created_by) VALUES ('Auto 2', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let worker1 = sqlx::query(
        "INSERT INTO task_worker_pcs (hostname, display_name, api_key_hash, status) VALUES ('host1', 'Worker 1', E'\\\\x1234', 'online') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let worker1_id: i64 = worker1.get("id");

    // RAII guard ensures cleanup on drop, even if assertions panic
    let _cleanup_guard = DashboardCleanupGuard {
        pool: pool.clone(),
        user_id,
        worker1_id,
        auto1_id,
    };

    // Batch seed task runs for test
    sqlx::query(
        r#"
        INSERT INTO task_runs (automation_id, worker_id, status, queued_at)
        VALUES
            ($1, $2, 'succeeded', CURRENT_TIMESTAMP),
            ($1, $2, 'failed', CURRENT_TIMESTAMP),
            ($1, $2, 'lost', CURRENT_TIMESTAMP)
        "#,
    )
    .bind(auto1_id)
    .bind(worker1_id)
    .execute(&pool)
    .await
    .unwrap();

    // Create session for user
    let session_id = Uuid::new_v4();
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, CURRENT_TIMESTAMP + INTERVAL '1 hour')")
        .bind(session_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();

    let config = Config::from_env().unwrap();
    let storage = deskdispatch::storage::StorageService::new(&config.s3_bucket).with_credentials(
        "key",
        "secret",
        "us-east-1",
        None::<String>,
    );

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool.clone(),
        storage,
        config: config.clone(),
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify,
        wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let app = deskdispatch::build_router(state);

    let cookie_header = format!("session_id={}", session_id);
    let req = Request::builder()
        .uri("/")
        .header(header::COOKIE, &cookie_header)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body_bytes);

    assert!(html.contains("Active Automations"));
    assert!(html.contains("Workers Online"));
    assert!(html.contains("Runs Today"));
    assert!(html.contains("Failed / Lost Runs Today"));

    // Verify stats cards rendered values
    assert!(html.contains("<div class=\"stat-label\">Active Automations</div>"));
    assert!(html.contains("<div class=\"stat-label\">Workers Online</div>"));
    assert!(html.contains("<div class=\"stat-label\">Runs Today</div>"));
    assert!(html.contains("<div class=\"stat-label\">Failed / Lost Runs Today</div>"));

    // Verify Recent Runs table
    assert!(html.contains("Recent Runs"));
    assert!(html.contains("Run ID"));
    assert!(html.contains("Automation"));
    assert!(html.contains("Worker"));
    assert!(html.contains("Status"));
    assert!(html.contains("Triggered By"));
    assert!(html.contains("Queued At"));
    assert!(html.contains("Auto 1"));
    assert!(html.contains("Worker 1"));
    assert!(html.contains("succeeded"));
    assert!(html.contains("failed"));
    assert!(html.contains("lost"));
}
