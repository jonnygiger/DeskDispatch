#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use deskdispatch::{
    AppState,
    config::{Config, SecretString},
    routes::*,
};

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

struct RunNowCleanupGuard {
    pool: PgPool,
    user_id: i64,
    automation_ids: Vec<i64>,
    worker_group_ids: Vec<i64>,
}

impl Drop for RunNowCleanupGuard {
    fn drop(&mut self) {
        let options = self.pool.connect_options().clone();
        let user_id = self.user_id;
        let automation_ids = std::mem::take(&mut self.automation_ids);
        let worker_group_ids = std::mem::take(&mut self.worker_group_ids);

        let cleanup = async move {
            let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(std::time::Duration::from_secs(2))
                .connect_with((*options).clone())
                .await
            else {
                return;
            };
            if !automation_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM task_runs WHERE automation_id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM step_key_presses WHERE step_id IN (SELECT id FROM automation_steps WHERE automation_id = ANY($1))")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM automation_steps WHERE automation_id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
                let _ =
                    sqlx::query("DELETE FROM automation_parameters WHERE automation_id = ANY($1)")
                        .bind(&automation_ids)
                        .execute(&pool)
                        .await;
                let _ = sqlx::query("DELETE FROM automations WHERE id = ANY($1)")
                    .bind(&automation_ids)
                    .execute(&pool)
                    .await;
            }
            if !worker_group_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM worker_groups WHERE id = ANY($1)")
                    .bind(&worker_group_ids)
                    .execute(&pool)
                    .await;
            }
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

#[derive(sqlx::FromRow)]
struct TaskRunDbRow {
    automation_id: i64,
    target_worker_group_id: Option<i64>,
    parameter_overrides: Option<serde_json::Value>,
    dispatched_automation_json: Option<serde_json::Value>,
    status: String,
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
async fn test_run_now_with_worker_group_and_parameter_overrides() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_run_now_with_worker_group_and_parameter_overrides"
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
            "/automations/{id}",
            axum::routing::get(get_automation_detail_handler),
        )
        .route(
            "/automations/{id}/run-now",
            axum::routing::post(post_run_now_automation_handler),
        )
        .route("/runs/{id}", axum::routing::get(get_run_detail_handler))
        .route(
            "/runs/{id}/status-frame",
            axum::routing::get(get_run_status_frame_handler),
        )
        .route(
            "/runs/{id}/cancel",
            axum::routing::post(post_cancel_run_handler),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            deskdispatch::auth::csrf_middleware,
        ))
        .with_state(state);

    let (admin_id, admin_session) = create_test_user(
        &pool,
        &format!("run_now_admin_{}", uuid::Uuid::new_v4().simple()),
        "admin",
    )
    .await;
    let csrf_token = deskdispatch::auth::generate_csrf_token(
        uuid::Uuid::parse_str(&admin_session).unwrap(),
        config.session_secret.expose_secret(),
    );

    let mut cleanup_guard = RunNowCleanupGuard {
        pool: pool.clone(),
        user_id: admin_id,
        automation_ids: Vec::new(),
        worker_group_ids: Vec::new(),
    };

    // 1. Create worker group
    let worker_group_id: i64 = sqlx::query_scalar(
        "INSERT INTO worker_groups (name, description) VALUES ($1, 'Test Group') RETURNING id",
    )
    .bind(format!("WG_{}", uuid::Uuid::new_v4().simple()))
    .fetch_one(&pool)
    .await
    .unwrap();
    cleanup_guard.worker_group_ids.push(worker_group_id);

    // 2. Create active automation
    let automation_id: i64 = sqlx::query_scalar("INSERT INTO automations (name, description, status, created_by) VALUES ('Overridden Automation', 'Desc', 'active', $1) RETURNING id")
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    cleanup_guard.automation_ids.push(automation_id);

    // 3. Add parameter to automation
    sqlx::query("INSERT INTO automation_parameters (automation_id, name, param_type, default_value, description) VALUES ($1, 'max_retries', 'int', '3', 'Max Retry Count')")
        .bind(automation_id)
        .execute(&pool)
        .await
        .unwrap();

    // 4. Add step to automation (active step required for Run Now)
    let step_id: i64 = sqlx::query_scalar("INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, 10.0, 'key_press', 'Press Key', 0) RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_key_presses (step_id, key_combo) VALUES ($1, 'enter')")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();

    // 5. GET /automations/{id} detail page and check parameter and worker group inputs exist
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
    assert!(body_str.contains("Run Now (Manual Trigger)"));
    assert!(body_str.contains("Target Worker Group"));
    assert!(body_str.contains("Parameter Overrides"));
    assert!(body_str.contains("max_retries"));

    // 6. POST /automations/{id}/run-now with target_worker_group_id and parameter override max_retries=10
    let run_now_body = format!(
        "worker_group_id={}&parameters[max_retries]=10&csrf_token={}",
        worker_group_id, csrf_token
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/automations/{}/run-now", automation_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(run_now_body))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let location = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        location.starts_with("/runs/"),
        "Expected redirect to /runs/{{id}}, got {}",
        location
    );

    let task_run_id: i64 = location.trim_start_matches("/runs/").parse().unwrap();

    // 7. Verify task_run DB row
    let run_db_row: TaskRunDbRow = sqlx::query_as("SELECT automation_id, target_worker_group_id, parameter_overrides, dispatched_automation_json, status FROM task_runs WHERE id = $1")
        .bind(task_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(run_db_row.automation_id, automation_id);
    assert_eq!(run_db_row.target_worker_group_id, Some(worker_group_id));
    assert_eq!(run_db_row.status, "queued");

    let overrides_obj = run_db_row
        .parameter_overrides
        .expect("Expected parameter_overrides JSON");
    assert_eq!(
        overrides_obj.get("max_retries").and_then(|v| v.as_str()),
        Some("10")
    );

    let dispatched_obj = run_db_row
        .dispatched_automation_json
        .expect("Expected dispatched_automation_json");
    let params_val = dispatched_obj
        .get("parameters")
        .expect("Expected parameters in dispatched json");
    assert_eq!(
        params_val.get("max_retries").and_then(|v| v.as_i64()),
        Some(10)
    );

    // 8. GET /runs/{task_run_id} and check execution parameters section rendered
    let req = Request::builder()
        .method("GET")
        .uri(format!("/runs/{}", task_run_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Execution Parameters"));
    assert!(body_str.contains("max_retries"));
    assert!(body_str.contains("10"));
    assert!(body_str.contains("Override"));

    // 9. Cancel task run (queued -> cancelled)
    let cancel_body = format!("csrf_token={}", csrf_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/runs/{}/cancel", task_run_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(cancel_body.clone()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let cancel_status: String = sqlx::query_scalar("SELECT status FROM task_runs WHERE id = $1")
        .bind(task_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cancel_status, "cancelled");

    // 10. Test running -> cancelling state display
    let task_run2_id: i64 = sqlx::query_scalar("INSERT INTO task_runs (automation_id, status, queued_at, started_at) VALUES ($1, 'running', now(), now()) RETURNING id")
        .bind(automation_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    // Cancel running task run -> transitions to 'cancelling'
    let req = Request::builder()
        .method("POST")
        .uri(format!("/runs/{}/cancel", task_run2_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(cancel_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let run2_status: String = sqlx::query_scalar("SELECT status FROM task_runs WHERE id = $1")
        .bind(task_run2_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run2_status, "cancelling");

    // GET /runs/{id}/status-frame and verify "Cancelling..." is rendered
    let req = Request::builder()
        .method("GET")
        .uri(format!("/runs/{}/status-frame", task_run2_id))
        .header(header::COOKIE, format!("session_id={}", admin_session))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Cancelling..."));
    assert!(body_str.contains("cancelling"));
}
