#![allow(clippy::unwrap_used)]
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::AppState;
use deskdispatch::auth::LoginRateLimiter;
use deskdispatch::config::{Config, SecretString};

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
        let options = self.pool.connect_options().clone();
        let user_id = self.user_id;
        let worker1_id = self.worker1_id;
        let auto1_id = self.auto1_id;

        let cleanup = async move {
            let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(std::time::Duration::from_secs(2))
                .connect_with((*options).clone())
                .await
            else {
                return;
            };
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
            rt.shutdown_timeout(std::time::Duration::from_millis(500));
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

    let user_id: i64 = sqlx::query_scalar(
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

    // Insert automations and worker
    let auto1_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, status, created_by) VALUES ('Auto 1', 'active', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let _auto2_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, status, created_by) VALUES ('Auto 2', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let worker1_id: i64 = sqlx::query_scalar(
        "INSERT INTO task_worker_pcs (hostname, display_name, api_key_hash, status) VALUES ('host1', 'Worker 1', E'\\\\x1234', 'online') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

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

    let config = create_test_config();
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
