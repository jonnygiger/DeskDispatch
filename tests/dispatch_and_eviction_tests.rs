use deskdispatch::routes::api_workers::sweep_stalled_task_runs;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tokio::task::JoinSet;

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    PgPool::connect(&db_url).await.ok()
}

async fn seed_test_user(pool: &PgPool) -> i64 {
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, 'admin') RETURNING id",
    )
    .bind(&username)
    .bind(&username)
    .fetch_one(pool)
    .await
    .unwrap();
    row.get("id")
}

async fn seed_test_automation(pool: &PgPool, user_id: i64) -> i64 {
    let row = sqlx::query(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Test Dispatch Auto', 'Desc', 'active', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();
    row.get("id")
}

async fn seed_test_worker(pool: &PgPool, hostname: &str) -> i64 {
    let row = sqlx::query(
        "INSERT INTO task_worker_pcs (hostname, display_name, status, last_heartbeat_at) VALUES ($1, $1, 'online', now()) RETURNING id",
    )
    .bind(hostname)
    .fetch_one(pool)
    .await
    .unwrap();
    row.get("id")
}

#[tokio::test]
async fn test_concurrent_dispatch_claiming() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_concurrent_dispatch_claiming");
        return;
    };

    let user_id = seed_test_user(&pool).await;
    let auto_id = seed_test_automation(&pool, user_id).await;

    // Seed 10 queued task runs
    let mut queued_run_ids = Vec::new();
    for _ in 0..10 {
        let row = sqlx::query(
            "INSERT INTO task_runs (automation_id, status, queued_at) VALUES ($1, 'queued', now()) RETURNING id",
        )
        .bind(auto_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let run_id: i64 = row.get("id");
        queued_run_ids.push(run_id);
    }

    // Seed 5 workers
    let mut worker_ids = Vec::new();
    for i in 1..=5 {
        let w_id = seed_test_worker(
            &pool,
            &format!("worker_{}_{}", i, uuid::Uuid::new_v4().simple()),
        )
        .await;
        worker_ids.push(w_id);
    }

    // Spawn concurrent claiming tasks using SELECT FOR UPDATE OF tr SKIP LOCKED
    let mut join_set = JoinSet::new();
    let shared_pool = Arc::new(pool.clone());

    for &worker_id in &worker_ids {
        let pool_clone = Arc::clone(&shared_pool);
        join_set.spawn(async move {
            let mut claimed_ids = Vec::new();

            // Attempt claiming multiple times per worker
            for _ in 0..10 {
                let mut tx = pool_clone.begin().await.unwrap();

                let select_res = sqlx::query(
                    r#"
                    SELECT tr.id AS task_run_id, tr.automation_id
                    FROM task_runs tr
                    LEFT JOIN schedules s ON tr.schedule_id = s.id
                    WHERE tr.status = 'queued'
                      AND (
                        tr.schedule_id IS NULL
                        OR s.worker_group_id IS NULL
                        OR EXISTS (
                          SELECT 1 FROM worker_group_members wgm
                          WHERE wgm.worker_id = $1 AND wgm.group_id = s.worker_group_id
                        )
                      )
                    ORDER BY tr.queued_at ASC, tr.id ASC
                    LIMIT 1
                    FOR UPDATE OF tr SKIP LOCKED
                    "#,
                )
                .bind(worker_id)
                .fetch_optional(&mut *tx)
                .await
                .unwrap();

                if let Some(row) = select_res {
                    let task_run_id: i64 = row.get("task_run_id");

                    sqlx::query(
                        r#"
                        UPDATE task_runs
                        SET status = 'running',
                            worker_id = $1,
                            started_at = now()
                        WHERE id = $2
                        "#,
                    )
                    .bind(worker_id)
                    .bind(task_run_id)
                    .execute(&mut *tx)
                    .await
                    .unwrap();

                    tx.commit().await.unwrap();
                    claimed_ids.push(task_run_id);
                } else {
                    tx.commit().await.unwrap();
                }
            }

            claimed_ids
        });
    }

    let mut total_claimed = Vec::new();
    while let Some(res) = join_set.join_next().await {
        let claimed_by_worker = res.unwrap();
        total_claimed.extend(claimed_by_worker);
    }

    // Verify exactly 10 task runs were claimed in total
    assert_eq!(
        total_claimed.len(),
        10,
        "Expected all 10 task runs to be claimed across workers"
    );

    // Verify every claimed ID is unique (no duplicate claims / race conditions)
    let mut sorted_claimed = total_claimed.clone();
    sorted_claimed.sort_unstable();
    sorted_claimed.dedup();
    assert_eq!(
        sorted_claimed.len(),
        10,
        "Claimed task run IDs should be strictly unique"
    );

    // Verify in database that all 10 queued_run_ids are now 'running' and assigned to workers
    for run_id in queued_run_ids {
        let (status, assigned_worker_id): (String, Option<i64>) =
            sqlx::query_as("SELECT status, worker_id FROM task_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(status, "running");
        assert!(assigned_worker_id.is_some());
        assert!(worker_ids.contains(&assigned_worker_id.unwrap()));
    }
}

#[tokio::test]
async fn test_stalled_task_run_eviction() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_stalled_task_run_eviction");
        return;
    };

    let user_id = seed_test_user(&pool).await;
    let auto_id = seed_test_automation(&pool, user_id).await;

    // Seed worker 1: active (heartbeat 10s ago)
    let active_worker_id = seed_test_worker(
        &pool,
        &format!("active_w_{}", uuid::Uuid::new_v4().simple()),
    )
    .await;
    sqlx::query("UPDATE task_worker_pcs SET last_heartbeat_at = now() - INTERVAL '10 seconds' WHERE id = $1")
        .bind(active_worker_id)
        .execute(&pool)
        .await
        .unwrap();

    // Seed worker 2: stalled (heartbeat 120s ago > 90s threshold)
    let stalled_worker_id = seed_test_worker(
        &pool,
        &format!("stalled_w_{}", uuid::Uuid::new_v4().simple()),
    )
    .await;
    sqlx::query("UPDATE task_worker_pcs SET last_heartbeat_at = now() - INTERVAL '120 seconds' WHERE id = $1")
        .bind(stalled_worker_id)
        .execute(&pool)
        .await
        .unwrap();

    // Create task run 1 assigned to active worker (status = running)
    let active_run_row = sqlx::query(
        "INSERT INTO task_runs (automation_id, worker_id, status, started_at) VALUES ($1, $2, 'running', now()) RETURNING id",
    )
    .bind(auto_id)
    .bind(active_worker_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let active_run_id: i64 = active_run_row.get("id");

    // Create task run 2 assigned to stalled worker (status = running)
    let stalled_run_row = sqlx::query(
        "INSERT INTO task_runs (automation_id, worker_id, status, started_at) VALUES ($1, $2, 'running', now()) RETURNING id",
    )
    .bind(auto_id)
    .bind(stalled_worker_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let stalled_run_id: i64 = stalled_run_row.get("id");

    // Execute sweep_stalled_task_runs
    let swept_count = sweep_stalled_task_runs(&pool)
        .await
        .expect("Sweeper failed");
    assert!(swept_count >= 1, "At least 1 stalled run should be evicted");

    // Verify active task run remains 'running'
    let (active_status, active_err): (String, Option<String>) =
        sqlx::query_as("SELECT status, error_message FROM task_runs WHERE id = $1")
            .bind(active_run_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(active_status, "running");
    assert!(active_err.is_none());

    // Verify stalled task run was transitioned to 'lost'
    let (stalled_status, stalled_err, completed_at): (
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as("SELECT status, error_message, completed_at FROM task_runs WHERE id = $1")
        .bind(stalled_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(stalled_status, "lost");
    assert_eq!(
        stalled_err.as_deref(),
        Some("Worker heartbeat lost (stalled execution)")
    );
    assert!(completed_at.is_some());
}

#[tokio::test]
async fn test_sweep_stalled_recording_sessions() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_sweep_stalled_recording_sessions");
        return;
    };

    let user_id = seed_test_user(&pool).await;

    // Seed active worker (heartbeat 10s ago)
    let active_worker_id = seed_test_worker(
        &pool,
        &format!("active_rec_w_{}", uuid::Uuid::new_v4().simple()),
    )
    .await;
    sqlx::query("UPDATE task_worker_pcs SET last_heartbeat_at = now() - INTERVAL '10 seconds' WHERE id = $1")
        .bind(active_worker_id)
        .execute(&pool)
        .await
        .unwrap();

    // Seed stalled worker (heartbeat 120s ago)
    let stalled_worker_id = seed_test_worker(
        &pool,
        &format!("stalled_rec_w_{}", uuid::Uuid::new_v4().simple()),
    )
    .await;
    sqlx::query("UPDATE task_worker_pcs SET last_heartbeat_at = now() - INTERVAL '120 seconds' WHERE id = $1")
        .bind(stalled_worker_id)
        .execute(&pool)
        .await
        .unwrap();

    // Create recording session for active worker
    let active_rec_row = sqlx::query(
        "INSERT INTO recording_sessions (worker_id, started_by_user_id, status, started_at) VALUES ($1, $2, 'recording', now()) RETURNING id",
    )
    .bind(active_worker_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let active_rec_id: i64 = active_rec_row.get("id");

    // Create recording session for stalled worker
    let stalled_rec_row = sqlx::query(
        "INSERT INTO recording_sessions (worker_id, started_by_user_id, status, started_at) VALUES ($1, $2, 'recording', now()) RETURNING id",
    )
    .bind(stalled_worker_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let stalled_rec_id: i64 = stalled_rec_row.get("id");

    // Execute background sweeper
    let swept_count = sweep_stalled_task_runs(&pool)
        .await
        .expect("Sweeper failed");
    assert!(
        swept_count >= 1,
        "At least 1 stalled record/run should be swept"
    );

    // Verify active worker's recording session remains 'recording'
    let active_rec_status: String =
        sqlx::query_scalar("SELECT status FROM recording_sessions WHERE id = $1")
            .bind(active_rec_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active_rec_status, "recording");

    // Verify stalled worker's recording session was transitioned to 'discarded'
    let stalled_rec_status: String =
        sqlx::query_scalar("SELECT status FROM recording_sessions WHERE id = $1")
            .bind(stalled_rec_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stalled_rec_status, "discarded");
}
