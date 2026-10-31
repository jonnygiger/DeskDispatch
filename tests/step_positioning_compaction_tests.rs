#![allow(clippy::unwrap_used)]
use deskdispatch::routes::automations::{
    check_and_compact_positions, compact_positions, reorder_step,
};
use sqlx::PgPool;

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

struct StepPositioningCleanupGuard {
    pool: PgPool,
    user_id: Option<i64>,
    automation_id: Option<i64>,
}

impl Drop for StepPositioningCleanupGuard {
    fn drop(&mut self) {
        let options = self.pool.connect_options().clone();
        let user_id = self.user_id;
        let auto_id = self.automation_id;

        let cleanup = async move {
            let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(std::time::Duration::from_secs(2))
                .connect_with((*options).clone())
                .await
            else {
                return;
            };
            if let Some(a_id) = auto_id {
                let _ = sqlx::query("DELETE FROM automation_steps WHERE automation_id = $1")
                    .bind(a_id)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM automations WHERE id = $1")
                    .bind(a_id)
                    .execute(&pool)
                    .await;
            }
            if let Some(u_id) = user_id {
                let _ = sqlx::query("DELETE FROM users WHERE id = $1")
                    .bind(u_id)
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
                rt.shutdown_timeout(std::time::Duration::from_millis(500));
            }
        })
        .join();
    }
}

async fn create_test_user_and_automation(pool: &PgPool) -> (i64, i64, StepPositioningCleanupGuard) {
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES ($1, 'hash', $2, 'admin') RETURNING id",
    )
    .bind(&username)
    .bind(&username)
    .fetch_one(pool)
    .await
    .unwrap();

    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, description, status, created_by) VALUES ('Test Positioning', 'Desc', 'draft', $1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();

    let guard = StepPositioningCleanupGuard {
        pool: pool.clone(),
        user_id: Some(user_id),
        automation_id: Some(auto_id),
    };

    (user_id, auto_id, guard)
}

#[tokio::test]
async fn test_sparse_position_assignment_and_reordering() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_sparse_position_assignment_and_reordering");
        return;
    };

    let (_user_id, auto_id, _guard) = create_test_user_and_automation(&pool).await;

    // 1. Insert initial steps sequentially with strongly typed query_scalar
    let step1_pos = 10.0;
    let step1_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'key_press', 'Step 1', 0) RETURNING id",
    )
    .bind(auto_id)
    .bind(step1_pos)
    .fetch_one(&pool)
    .await
    .unwrap();

    let max_pos: f64 =
        sqlx::query_scalar("SELECT MAX(position) FROM automation_steps WHERE automation_id = $1")
            .bind(auto_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let step2_pos = max_pos + 10.0;
    let step2_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'key_press', 'Step 2', 0) RETURNING id",
    )
    .bind(auto_id)
    .bind(step2_pos)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step1_pos, 10.0);
    assert_eq!(step2_pos, 20.0);

    // 2. Insert midpoint step between Step 1 (10.0) and Step 2 (20.0) -> position 15.0
    let midpoint_pos = (step1_pos + step2_pos) / 2.0;
    let step_mid_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, $2, 'key_press', 'Midpoint Step', 0) RETURNING id",
    )
    .bind(auto_id)
    .bind(midpoint_pos)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(midpoint_pos, 15.0);

    let step_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(step_ids, vec![step1_id, step_mid_id, step2_id]);

    // 3. Test check_and_compact_positions when gap is wide (10.0, 15.0, 20.0 - min gap 5.0 >= 0.0001)
    let _ = check_and_compact_positions(&pool, auto_id).await;

    let positions_before: Vec<f64> = sqlx::query_scalar(
        "SELECT position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(positions_before, vec![10.0, 15.0, 20.0]);

    // 4. Test explicit compaction resets positions to clean 10.0 increments
    let _ = compact_positions(&pool, auto_id).await;

    let positions_after: Vec<f64> = sqlx::query_scalar(
        "SELECT position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(positions_after, vec![10.0, 20.0, 30.0]);
}

#[tokio::test]
async fn test_gap_precision_threshold_and_compaction_trigger() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_gap_precision_threshold_and_compaction_trigger"
        );
        return;
    };

    let (_user_id, auto_id, _guard) = create_test_user_and_automation(&pool).await;

    // Batch insert steps with a gap less than 0.0001 (e.g. 10.0 and 10.00005) in a single query
    sqlx::query(
        r#"
        INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms)
        SELECT $1, pos, 'key_press', label, 0
        FROM UNNEST($2::float8[], $3::text[]) AS u(pos, label)
        "#,
    )
    .bind(auto_id)
    .bind([10.0f64, 10.00005f64, 20.0f64])
    .bind(["Step A", "Step B", "Step C"])
    .execute(&pool)
    .await
    .unwrap();

    // Verify initial positions before check
    let pos_before: Vec<f64> = sqlx::query_scalar(
        "SELECT position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(pos_before.len(), 3);
    assert!((pos_before[1] - pos_before[0]) < 0.0001);

    // Call check_and_compact_positions - should trigger compaction
    check_and_compact_positions(&pool, auto_id).await.unwrap();

    // Verify positions have been compacted to 10.0, 20.0, 30.0
    let pos_after: Vec<f64> = sqlx::query_scalar(
        "SELECT position FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(pos_after, vec![10.0, 20.0, 30.0]);
}

#[tokio::test]
async fn test_reorder_step_swaps_positions_without_unique_constraint_error() {
    let Some(pool) = get_test_pool().await else {
        println!(
            "Database not available, skipping test_reorder_step_swaps_positions_without_unique_constraint_error"
        );
        return;
    };

    let (_user_id, auto_id, _guard) = create_test_user_and_automation(&pool).await;

    // Insert 2 adjacent steps using query_scalar
    let step1_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, 10.0, 'key_press', 'Step 1', 0) RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let step2_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label, post_delay_ms) VALUES ($1, 20.0, 'key_press', 'Step 2', 0) RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Move step 2 UP (should swap position with step 1)
    let reorder_res = reorder_step(&pool, auto_id, step2_id, true).await;
    assert!(
        reorder_res.is_ok(),
        "reorder_step failed: {:?}",
        reorder_res.err()
    );

    let ordered_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM automation_steps WHERE automation_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(auto_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(ordered_ids, vec![step2_id, step1_id]);
}
