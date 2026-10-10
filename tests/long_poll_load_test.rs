#![allow(clippy::unwrap_used)]
use axum::{extract::State, response::IntoResponse};
use deskdispatch::{
    AppState, auth::WorkerAuth, config::Config, routes::api_workers::get_next_assignment_handler,
};
use sqlx::postgres::PgPoolOptions;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn test_long_poll_300_workers_no_pool_exhaustion() {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            eprintln!(
                "Skipping test_long_poll_300_workers_no_pool_exhaustion: DATABASE_URL not set"
            );
            return;
        }
    };

    // Configure a small pool (5 max connections) to ensure 300 long-polling workers don't hog connections
    let pool = match PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&db_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Skipping test_long_poll_300_workers_no_pool_exhaustion: failed to connect to DB: {}",
                e
            );
            return;
        }
    };

    // Run migrations to ensure triggers exist
    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        eprintln!("Failed to run migrations: {}", e);
        return;
    }

    let mut config = Config::from_env().unwrap_or_else(|_| Config {
        database_url: db_url.clone(),
        public_base_url: "http://localhost:3000".to_string(),
        s3_endpoint: None,
        s3_public_endpoint: None,
        s3_bucket: "deskdispatch-bucket".to_string(),
        s3_access_key: "rustfsadmin".to_string(),
        s3_secret_key: secrecy::SecretString::from("rustfsadminpassword".to_string()),
        s3_region: "us-east-1".to_string(),
        session_secret: secrecy::SecretString::from("test_secret".to_string()),
        bind_address: "0.0.0.0:3000".to_string(),
        environment: "development".to_string(),
        min_agent_version: None,
        worker_poll_interval_secs: 5,
        worker_heartbeat_interval_secs: 15,
        trust_proxy_headers: false,
        database_max_connections: 5,
        database_acquire_timeout_secs: 5,
        worker_long_poll_timeout_secs: 2,
        session_retention_days: 7,
        audit_log_retention_days: 90,
        task_run_step_retention_days: 30,
        screenshot_retention_days: 30,
        metrics_bind_address: None,
    });
    config.worker_long_poll_timeout_secs = 2; // Short timeout for fast test execution

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(500);

    let state = AppState {
        db: pool.clone(),
        s3_client: aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::new()
                .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
                .build(),
        ),
        config,
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tx_notify.clone(),
    };

    // Create a dummy worker row in task_worker_pcs
    let worker_id: i64 = match sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, status, api_key_hash)
        VALUES ('load-test-host', 'Load Test Worker', 'idle', $1)
        RETURNING id
        "#,
    )
    .bind(vec![0u8; 32])
    .fetch_one(&pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Failed to insert test worker: {}", e);
            return;
        }
    };

    let auth_worker = WorkerAuth {
        id: worker_id,
        hostname: "load-test-host".to_string(),
        display_name: "Load Test Worker".to_string(),
        status: "idle".to_string(),
        last_heartbeat_at: None,
        screen_width: None,
        screen_height: None,
        os_info: None,
        agent_version: None,
        created_at: chrono::Utc::now(),
    };

    // Spawn 300 concurrent workers calling get_next_assignment_handler
    let num_workers = 300;
    let mut handles = Vec::with_capacity(num_workers);

    for _ in 0..num_workers {
        let state_clone = state.clone();
        let worker_clone = auth_worker.clone();
        let handle = tokio::spawn(async move {
            let res = get_next_assignment_handler(worker_clone, State(state_clone)).await;
            res.into_response().status().is_success()
        });
        handles.push(handle);
    }

    // Ensure all 300 workers complete within 8 seconds (well within timeout and without connection pool error)
    let results = match timeout(Duration::from_secs(8), async {
        let mut res = Vec::with_capacity(handles.len());
        for h in handles {
            res.push(h.await);
        }
        res
    })
    .await
    {
        Ok(res) => res,
        Err(_) => panic!(
            "Long poll load test timed out! Possible connection pool exhaustion or deadlock."
        ),
    };

    for res in results {
        let success: bool = res.expect("Task panicked");
        assert!(success, "Worker long poll request failed");
    }

    // Cleanup
    let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = $1")
        .bind(worker_id)
        .execute(&pool)
        .await;
}
