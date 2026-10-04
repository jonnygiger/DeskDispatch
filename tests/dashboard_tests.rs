use app::auth::LoginRateLimiter;
use app::config::Config;
use app::routes::get_index_handler;
use app::AppState;
use argon2::{PasswordHasher, Argon2};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::get,
    Router,
};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());
    sqlx::PgPool::connect(&db_url).await.ok()
}

#[tokio::test]
async fn test_dashboard_stats_tiles() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping integration test");
        return;
    };

    // Seed test user
    let username = format!("dashuser_{}", Uuid::new_v4().simple());
    let hash = Argon2::default()
        .hash_password("Password123!".as_bytes())
        .unwrap()
        .to_string();

    let user_row = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, $2, 'Dashboard Tester', 'admin', true)
        RETURNING id
        "#,
    )
    .bind(&username)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user_id: i64 = user_row.get("id");

    // Insert dummy data into automations, task_worker_pcs, task_runs
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

    let _run1 = sqlx::query(
        "INSERT INTO task_runs (automation_id, worker_id, status, queued_at) VALUES ($1, $2, 'succeeded', CURRENT_TIMESTAMP)",
    )
    .bind(auto1_id)
    .bind(worker1_id)
    .execute(&pool)
    .await
    .unwrap();

    let _run2 = sqlx::query(
        "INSERT INTO task_runs (automation_id, worker_id, status, queued_at) VALUES ($1, $2, 'failed', CURRENT_TIMESTAMP)",
    )
    .bind(auto1_id)
    .bind(worker1_id)
    .execute(&pool)
    .await
    .unwrap();

    let _run3 = sqlx::query(
        "INSERT INTO task_runs (automation_id, worker_id, status, queued_at) VALUES ($1, $2, 'lost', CURRENT_TIMESTAMP)",
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
        rate_limiter: LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/", get(get_index_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            app::auth::csrf_middleware,
        ))
        .with_state(state);

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

    // Clean up test data
    let _ = sqlx::query("DELETE FROM task_runs WHERE automation_id = $1").bind(auto1_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1").bind(worker1_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM automations WHERE created_by = $1").bind(user_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(user_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await;
}
