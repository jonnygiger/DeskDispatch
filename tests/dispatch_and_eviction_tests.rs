#![allow(clippy::unwrap_used)]
use deskdispatch::routes::api_workers::sweep_stalled_task_runs;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::task::JoinSet;

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

struct DispatchCleanupGuard {
    pool: PgPool,
    user_ids: Vec<i64>,
    auto_ids: Vec<i64>,
    worker_ids: Vec<i64>,
    worker_group_ids: Vec<i64>,
    task_run_ids: Vec<i64>,
    recording_session_ids: Vec<i64>,
}

impl DispatchCleanupGuard {
    fn new(pool: PgPool) -> Self {
        Self {
            pool,
            user_ids: Vec::new(),
            auto_ids: Vec::new(),
            worker_ids: Vec::new(),
            worker_group_ids: Vec::new(),
            task_run_ids: Vec::new(),
            recording_session_ids: Vec::new(),
        }
    }
}

impl Drop for DispatchCleanupGuard {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let user_ids = self.user_ids.clone();
        let auto_ids = self.auto_ids.clone();
        let worker_ids = self.worker_ids.clone();
        let worker_group_ids = self.worker_group_ids.clone();
        let task_run_ids = self.task_run_ids.clone();
        let recording_session_ids = self.recording_session_ids.clone();

        let cleanup = async move {
            if !recording_session_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM recording_sessions WHERE id = ANY($1)")
                    .bind(&recording_session_ids)
                    .execute(&pool)
                    .await;
            }
            if !task_run_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM task_runs WHERE id = ANY($1)")
                    .bind(&task_run_ids)
                    .execute(&pool)
                    .await;
            }
            if !worker_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM worker_group_members WHERE worker_id = ANY($1)")
                    .bind(&worker_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE id = ANY($1)")
                    .bind(&worker_ids)
                    .execute(&pool)
                    .await;
            }
            if !worker_group_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM worker_groups WHERE id = ANY($1)")
                    .bind(&worker_group_ids)
                    .execute(&pool)
                    .await;
            }
            if !auto_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM automations WHERE id = ANY($1)")
                    .bind(&auto_ids)
                    .execute(&pool)
                    .await;
            }
            if !user_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM users WHERE id = ANY($1)")
                    .bind(&user_ids)
                    .execute(&pool)
                    .await;
            }
        };

        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(cleanup);
            }
        })
        .join();
    }
}

async fn seed_test_user(pool: &PgPool) -> i64 {
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, 'admin') RETURNING id",
    )
    .bind(&username)
    .bind(&username)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_test_automation(pool: &PgPool, user_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Test Dispatch Auto', 'Desc', 'active', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_test_worker(pool: &PgPool, hostname: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO task_worker_pcs (hostname, display_name, status, last_heartbeat_at) VALUES ($1, $1, 'online', now()) RETURNING id",
    )
    .bind(hostname)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn test_concurrent_dispatch_claiming() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_concurrent_dispatch_claiming");
        return;
    };

    let user_id = seed_test_user(&pool).await;
    let auto_id = seed_test_automation(&pool, user_id).await;

    let mut cleanup_guard = DispatchCleanupGuard::new(pool.clone());
    cleanup_guard.user_ids.push(user_id);
    cleanup_guard.auto_ids.push(auto_id);

    // Seed worker group A
    let group_a_name = format!("GroupA_{}", uuid::Uuid::new_v4().simple());
    let group_a_id: i64 = sqlx::query_scalar(
        "INSERT INTO worker_groups (name, description) VALUES ($1, 'Group A Desc') RETURNING id",
    )
    .bind(&group_a_name)
    .fetch_one(&pool)
    .await
    .unwrap();

    cleanup_guard.worker_group_ids.push(group_a_id);

    // Seed 5 workers
    let hostnames: Vec<String> = (1..=5)
        .map(|i| format!("worker_{}_{}", i, uuid::Uuid::new_v4().simple()))
        .collect();

    let worker_ids: Vec<i64> = sqlx::query_scalar(
        r#"
        INSERT INTO task_worker_pcs (hostname, display_name, status, last_heartbeat_at)
        SELECT h, h, 'online', now()
        FROM UNNEST($1::text[]) AS h
        RETURNING id
        "#,
    )
    .bind(&hostnames)
    .fetch_all(&pool)
    .await
    .unwrap();

    cleanup_guard
        .worker_ids
        .extend(worker_ids.iter().copied());

    let group_a_worker_ids = vec![worker_ids[0], worker_ids[1]];

    // Add Workers 1 and 2 to Group A
    for &w_id in &group_a_worker_ids {
        sqlx::query(
            "INSERT INTO worker_group_members (group_id, worker_id) VALUES ($1, $2)",
        )
        .bind(group_a_id)
        .bind(w_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Batch seed 5 targeted task runs (targeting group_a_id)
    let auto_ids_5 = vec![auto_id; 5];
    let group_ids_5 = vec![group_a_id; 5];

    let targeted_run_ids: Vec<i64> = sqlx::query_scalar(
        r#"
        INSERT INTO task_runs (automation_id, target_worker_group_id, status, queued_at)
        SELECT a_id, g_id, 'queued', now()
        FROM UNNEST($1::int8[], $2::int8[]) AS u(a_id, g_id)
        RETURNING id
        "#,
    )
    .bind(&auto_ids_5)
    .bind(&group_ids_5)
    .fetch_all(&pool)
    .await
    .unwrap();

    // Batch seed 5 untargeted task runs (target_worker_group_id IS NULL)
    let untargeted_run_ids: Vec<i64> = sqlx::query_scalar(
        r#"
        INSERT INTO task_runs (automation_id, status, queued_at)
        SELECT a_id, 'queued', now()
        FROM UNNEST($1::int8[]) AS a_id
        RETURNING id
        "#,
    )
    .bind(&auto_ids_5)
    .fetch_all(&pool)
    .await
    .unwrap();

    let all_queued_run_ids: Vec<i64> = targeted_run_ids
        .iter()
        .chain(untargeted_run_ids.iter())
        .copied()
        .collect();

    cleanup_guard
        .task_run_ids
        .extend(all_queued_run_ids.iter().copied());

    // Spawn concurrent claiming tasks synchronized by a Barrier so all 5 workers enter claiming simultaneously
    let barrier = Arc::new(tokio::sync::Barrier::new(worker_ids.len()));
    let mut join_set = JoinSet::new();
    let shared_pool = Arc::new(pool.clone());

    for &worker_id in &worker_ids {
        let pool_clone = Arc::clone(&shared_pool);
        let barrier_clone = Arc::clone(&barrier);
        join_set.spawn(async move {
            barrier_clone.wait().await;
            let mut claimed_ids = Vec::new();

            // Attempt claiming multiple times per worker
            for _ in 0..10 {
                let mut tx = pool_clone.begin().await.unwrap();

                let select_res: Option<i64> = sqlx::query_scalar(
                    r#"
                    SELECT tr.id
                    FROM task_runs tr
                    LEFT JOIN schedules s ON tr.schedule_id = s.id
                    WHERE tr.status = 'queued'
                      AND (
                        COALESCE(tr.target_worker_group_id, s.worker_group_id) IS NULL
                        OR EXISTS (
                          SELECT 1 FROM worker_group_members wgm
                          WHERE wgm.worker_id = $1 AND wgm.group_id = COALESCE(tr.target_worker_group_id, s.worker_group_id)
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

                if let Some(task_run_id) = select_res {
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

    // Verify targeted task runs were ONLY claimed by Group A members (Workers 1 and 2)
    for run_id in targeted_run_ids {
        let (status, assigned_worker_id): (String, Option<i64>) =
            sqlx::query_as("SELECT status, worker_id FROM task_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(status, "running");
        assert!(
            assigned_worker_id.is_some(),
            "Targeted task run {} should be assigned",
            run_id
        );
        let assigned = assigned_worker_id.unwrap();
        assert!(
            group_a_worker_ids.contains(&assigned),
            "Targeted task run {} assigned to worker {} who is not in Group A",
            run_id,
            assigned
        );
    }

    // Verify untargeted task runs were claimed and assigned to valid online workers
    for run_id in untargeted_run_ids {
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

    let mut cleanup_guard = DispatchCleanupGuard::new(pool.clone());

    let user_id = seed_test_user(&pool).await;
    let auto_id = seed_test_automation(&pool, user_id).await;
    cleanup_guard.user_ids.push(user_id);
    cleanup_guard.auto_ids.push(auto_id);

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

    cleanup_guard.worker_ids.push(active_worker_id);
    cleanup_guard.worker_ids.push(stalled_worker_id);

    // Create task run 1 assigned to active worker (status = running)
    let active_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO task_runs (automation_id, worker_id, status, started_at) VALUES ($1, $2, 'running', now()) RETURNING id",
    )
    .bind(auto_id)
    .bind(active_worker_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Create task run 2 assigned to stalled worker (status = running)
    let stalled_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO task_runs (automation_id, worker_id, status, started_at) VALUES ($1, $2, 'running', now()) RETURNING id",
    )
    .bind(auto_id)
    .bind(stalled_worker_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    cleanup_guard.task_run_ids.push(active_run_id);
    cleanup_guard.task_run_ids.push(stalled_run_id);

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

    let mut cleanup_guard = DispatchCleanupGuard::new(pool.clone());

    let user_id = seed_test_user(&pool).await;
    cleanup_guard.user_ids.push(user_id);

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

    cleanup_guard.worker_ids.push(active_worker_id);
    cleanup_guard.worker_ids.push(stalled_worker_id);

    // Create recording session for active worker
    let active_rec_id: i64 = sqlx::query_scalar(
        "INSERT INTO recording_sessions (worker_id, started_by_user_id, status, started_at) VALUES ($1, $2, 'recording', now()) RETURNING id",
    )
    .bind(active_worker_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Create recording session for stalled worker
    let stalled_rec_id: i64 = sqlx::query_scalar(
        "INSERT INTO recording_sessions (worker_id, started_by_user_id, status, started_at) VALUES ($1, $2, 'recording', now()) RETURNING id",
    )
    .bind(stalled_worker_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    cleanup_guard.recording_session_ids.push(active_rec_id);
    cleanup_guard.recording_session_ids.push(stalled_rec_id);

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
