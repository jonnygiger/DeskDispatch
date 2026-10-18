#![allow(clippy::unwrap_used)]
use axum::{extract::State, response::IntoResponse};
use deskdispatch::{
    AppState, auth::WorkerAuth, config::Config, routes::api_workers::get_next_assignment_handler,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::time::{Duration, timeout};

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_millis(500))
        .connect(&db_url)
        .await
        .ok()?;

    let _ = sqlx::migrate!("./migrations").run(&pool).await;
    Some(pool)
}

struct WorkerCleanupGuard {
    pool: sqlx::PgPool,
    host_prefix: String,
}

impl Drop for WorkerCleanupGuard {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let prefix = format!("{}%", self.host_prefix);
        tokio::task::spawn(async move {
            let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE hostname LIKE $1")
                .bind(prefix)
                .execute(&pool)
                .await;
        });
    }
}

async fn cleanup_workers(pool: &sqlx::PgPool, host_prefix: &str) {
    let prefix = format!("{}%", host_prefix);
    let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE hostname LIKE $1")
        .bind(prefix)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn test_long_poll_300_workers_no_pool_exhaustion() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_long_poll_300_workers_no_pool_exhaustion");
        return;
    };

    let mut config = Config::from_env().unwrap_or_else(|_| Config {
        database_url: "postgres://postgres:postgrespassword@localhost:5432/deskdispatch"
            .to_string(),
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
        unattended_registration_psk: secrecy::SecretString::from(
            "deskdispatch_wps_psk_secret".to_string(),
        ),
    });
    config.database_max_connections = 5;
    config.database_acquire_timeout_secs = 5;
    config.worker_long_poll_timeout_secs = 2; // Short timeout for fast test execution

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(1000);

    let state = AppState {
        db: pool.clone(),
        storage: deskdispatch::storage::StorageService::new(&config.s3_bucket),
        config,
        rate_limiter: deskdispatch::auth::LoginRateLimiter::default(),
        task_queue_notifier: tx_notify.clone(),
        wps_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let run_id_suffix = uuid::Uuid::new_v4().simple().to_string();
    let host_prefix = format!("load-test-host-{}", run_id_suffix);

    // RAII guard guarantees worker cleanup even on assertion panic
    let _cleanup_guard = WorkerCleanupGuard {
        pool: pool.clone(),
        host_prefix: host_prefix.clone(),
    };

    // 1. Seed 300 distinct workers in task_worker_pcs
    let num_workers = 300;
    let mut auth_workers = Vec::with_capacity(num_workers);

    for i in 0..num_workers {
        let hostname = format!("{}-{}", host_prefix, i);
        let display_name = format!("Load Test Worker {}", i);
        let worker_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO task_worker_pcs (hostname, display_name, status, api_key_hash)
            VALUES ($1, $2, 'idle', $3)
            RETURNING id
            "#,
        )
        .bind(&hostname)
        .bind(&display_name)
        .bind(vec![0u8; 32])
        .fetch_one(&pool)
        .await
        .expect("Failed to insert test worker");

        auth_workers.push(WorkerAuth {
            id: worker_id,
            hostname,
            display_name,
            status: "idle".to_string(),
            last_heartbeat_at: None,
            screen_width: None,
            screen_height: None,
            os_info: None,
            agent_version: None,
            created_at: chrono::Utc::now(),
        });
    }

    // Scenario A: 300 distinct workers long-polling concurrently without holding connections
    let mut handles = Vec::with_capacity(num_workers);
    for worker in auth_workers.iter().cloned() {
        let state_clone = state.clone();
        let handle = tokio::spawn(async move {
            let res = get_next_assignment_handler(worker, State(state_clone)).await;
            res.into_response().status().is_success()
        });
        handles.push(handle);
    }

    // Ensure all 300 distinct workers complete long polling within 8s without pool exhaustion or deadlocks
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
            "Long poll load test timed out! Connection pool exhaustion or deadlock detected."
        ),
    };

    for (idx, res) in results.into_iter().enumerate() {
        let success: bool = res.expect("Worker tokio task panicked");
        assert!(
            success,
            "Worker {} long poll request failed or threw connection error",
            idx
        );
    }

    // Scenario B: Broadcast notification during active long polling claims queued task run immediately
    // Seed test user, active automation, and 1 queued task run
    let user_username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, 'admin') RETURNING id",
    )
    .bind(&user_username)
    .bind(&user_username)
    .fetch_one(&pool)
    .await
    .expect("Failed to seed user");

    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Load Test Auto', 'Desc', 'active', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to seed automation");

    let task_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO task_runs (automation_id, status, queued_at) VALUES ($1, 'queued', now()) RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to seed queued task_run");

    // Spawn 300 long polling workers again
    let mut handles_wake = Vec::with_capacity(num_workers);
    for worker in auth_workers.iter().cloned() {
        let state_clone = state.clone();
        let handle =
            tokio::spawn(
                async move { get_next_assignment_handler(worker, State(state_clone)).await },
            );
        handles_wake.push(handle);
    }

    // Give workers a tiny window to enter long polling wait on broadcast channel
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send task queue notification to wake up long-polling workers
    let _ = tx_notify.send(());

    // Collect responses
    let wake_results = match timeout(Duration::from_secs(8), async {
        let mut res = Vec::with_capacity(handles_wake.len());
        for h in handles_wake {
            res.push(h.await);
        }
        res
    })
    .await
    {
        Ok(res) => res,
        Err(_) => panic!(
            "Broadcast wake-up load test timed out! Connection pool deadlock under broadcast notification."
        ),
    };

    for res in wake_results {
        let res_inner = res.expect("Task panicked");
        assert!(
            res_inner.is_ok(),
            "Long poll request failed during broadcast wake-up: {:?}",
            res_inner.err()
        );
        // Look for assignment response vs none
        let response = res_inner.unwrap().into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    // Verify exactly 1 worker claimed the queued task run
    let (run_status, assigned_worker_id): (String, Option<i64>) =
        sqlx::query_as("SELECT status, worker_id FROM task_runs WHERE id = $1")
            .bind(task_run_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query task_run");

    assert_eq!(run_status, "running");
    assert!(
        assigned_worker_id.is_some(),
        "Expected task run to be assigned to a worker"
    );
    let worker_id_claimed = assigned_worker_id.unwrap();
    assert!(
        auth_workers.iter().any(|w| w.id == worker_id_claimed),
        "Claiming worker should be one of the 300 long-polling test workers"
    );

    // Explicit cleanup of test entities
    let _ = sqlx::query("DELETE FROM task_runs WHERE id = $1")
        .bind(task_run_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM automations WHERE id = $1")
        .bind(auto_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await;
    cleanup_workers(&pool, &host_prefix).await;
}
