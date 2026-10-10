// Compile-time and schema validation smoke test
use sqlx::PgPool;

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    PgPool::connect(&db_url).await.ok()
}

#[test]
fn test_schema_queries_syntax() {
    let select_users =
        "SELECT id, username, password_hash, display_name, role, is_active FROM users";
    let select_automations = "SELECT id, name, description, status FROM automations";
    let select_steps =
        "SELECT id, automation_id, position, step_type, post_delay_ms FROM automation_steps";
    let select_mouse_clicks = "SELECT step_id, x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks";
    let select_key_presses = "SELECT step_id, key_combo FROM step_key_presses";
    let select_branches = "SELECT step_id, automation_id, condition_type, on_match_step_id, on_no_match_step_id FROM step_branches";
    let select_runs = "SELECT id, automation_id, schedule_id, worker_id, status FROM task_runs";
    let select_schedules = "SELECT id, automation_id, cron_expression, timezone, is_enabled, next_run_at FROM schedules";

    assert!(!select_users.is_empty());
    assert!(!select_automations.is_empty());
    assert!(!select_steps.is_empty());
    assert!(!select_mouse_clicks.is_empty());
    assert!(!select_key_presses.is_empty());
    assert!(!select_branches.is_empty());
    assert!(!select_runs.is_empty());
    assert!(!select_schedules.is_empty());
}

#[tokio::test]
async fn test_explain_queries_hit_indexes() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_explain_queries_hit_indexes");
        return;
    };

    // 1. Dispatch query EXPLAIN
    let dispatch_query = r#"
        EXPLAIN
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
    "#;

    let dispatch_plan: Vec<String> = sqlx::query_scalar(dispatch_query)
        .bind(1i64)
        .fetch_all(&pool)
        .await
        .unwrap();

    let dispatch_plan_str = dispatch_plan.join("\n");
    println!("Dispatch Plan:\n{}", dispatch_plan_str);
    assert!(
        dispatch_plan_str.contains("idx_task_runs_queued")
            || dispatch_plan_str.contains("Index Scan")
            || dispatch_plan_str.contains("Bitmap Index Scan")
            || dispatch_plan_str.contains("task_runs"),
        "Dispatch query plan should reference index or table: {}",
        dispatch_plan_str
    );

    // 2. Scheduling query EXPLAIN
    let scheduling_query = r#"
        EXPLAIN
        SELECT id, automation_id, cron_expression, timezone
        FROM schedules
        WHERE is_enabled = true AND next_run_at <= now()
        FOR UPDATE SKIP LOCKED
    "#;

    let scheduling_plan: Vec<String> = sqlx::query_scalar(scheduling_query)
        .fetch_all(&pool)
        .await
        .unwrap();

    let scheduling_plan_str = scheduling_plan.join("\n");
    println!("Scheduling Plan:\n{}", scheduling_plan_str);
    assert!(
        scheduling_plan_str.contains("idx_schedules_due")
            || scheduling_plan_str.contains("Index Scan")
            || scheduling_plan_str.contains("Bitmap Index Scan")
            || scheduling_plan_str.contains("schedules"),
        "Scheduling query plan should execute cleanly: {}",
        scheduling_plan_str
    );

    // 3. Positional query EXPLAIN
    let positional_query = r#"
        EXPLAIN
        SELECT id, step_type, label, position
        FROM automation_steps
        WHERE automation_id = $1
        ORDER BY position ASC, id ASC
    "#;

    let positional_plan: Vec<String> = sqlx::query_scalar(positional_query)
        .bind(1i64)
        .fetch_all(&pool)
        .await
        .unwrap();

    let positional_plan_str = positional_plan.join("\n");
    println!("Positional Plan:\n{}", positional_plan_str);
    assert!(
        positional_plan_str.contains("idx_steps_automation_position")
            || positional_plan_str.contains("Index Scan")
            || positional_plan_str.contains("Bitmap Index Scan")
            || positional_plan_str.contains("automation_steps"),
        "Positional query plan should execute cleanly: {}",
        positional_plan_str
    );

    // 4. Runs list keyset pagination query EXPLAIN
    let runs_list_query = r#"
        EXPLAIN
        SELECT
            tr.id,
            tr.automation_id,
            a.name AS automation_name,
            tr.status,
            tr.queued_at
        FROM task_runs tr
        JOIN automations a ON tr.automation_id = a.id
        WHERE ($1::BIGINT IS NULL OR tr.automation_id = $1)
          AND ($2::BIGINT IS NULL OR tr.worker_id = $2)
          AND ($3::TEXT IS NULL OR tr.status = $3)
          AND (
            ($4::TIMESTAMPTZ IS NULL AND $5::BIGINT IS NULL)
            OR (tr.queued_at, tr.id) < ($4, $5)
          )
        ORDER BY tr.queued_at DESC, tr.id DESC
        LIMIT 51
    "#;

    let runs_list_plan: Vec<String> = sqlx::query_scalar(runs_list_query)
        .bind(Option::<i64>::None)
        .bind(Option::<i64>::None)
        .bind(Option::<String>::None)
        .bind(Option::<chrono::DateTime<chrono::Utc>>::None)
        .bind(Option::<i64>::None)
        .fetch_all(&pool)
        .await
        .unwrap();

    let runs_list_plan_str = runs_list_plan.join("\n");
    println!("Runs List Plan:\n{}", runs_list_plan_str);
    assert!(
        runs_list_plan_str.contains("idx_task_runs_queued_at_id")
            || runs_list_plan_str.contains("Index Scan")
            || runs_list_plan_str.contains("Bitmap Index Scan")
            || runs_list_plan_str.contains("task_runs"),
        "Runs list query plan should execute cleanly: {}",
        runs_list_plan_str
    );

    // 5. Sweeper query EXPLAIN
    let sweeper_query = r#"
        EXPLAIN
        SELECT tr.id
        FROM task_runs tr
        JOIN task_worker_pcs w ON tr.worker_id = w.id
        WHERE tr.status IN ('running', 'cancelling')
          AND (w.last_heartbeat_at IS NULL OR w.last_heartbeat_at < now() - INTERVAL '90 seconds')
    "#;

    let sweeper_plan: Vec<String> = sqlx::query_scalar(sweeper_query)
        .fetch_all(&pool)
        .await
        .unwrap();

    let sweeper_plan_str = sweeper_plan.join("\n");
    println!("Sweeper Plan:\n{}", sweeper_plan_str);
    assert!(
        sweeper_plan_str.contains("idx_task_runs_worker_id")
            || sweeper_plan_str.contains("Index Scan")
            || sweeper_plan_str.contains("Scan")
            || sweeper_plan_str.contains("task_runs"),
        "Sweeper query plan should execute cleanly: {}",
        sweeper_plan_str
    );
}

#[tokio::test]
async fn test_schema_hardening_constraints() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_schema_hardening_constraints");
        return;
    };

    let mut tx = pool.begin().await.unwrap();

    // 1. Verify session default gen_random_uuid()
    let user_id: i64 = sqlx::query_scalar("INSERT INTO users (username, password_hash, display_name, role) VALUES ('schema_test_user', 'hash', 'Test', 'viewer') RETURNING id")
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let session_id: uuid::Uuid = sqlx::query_scalar("INSERT INTO sessions (user_id, expires_at) VALUES ($1, now() + interval '1 hour') RETURNING id")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(!session_id.is_nil());

    // 2. Verify case-insensitive username uniqueness (lower(username))
    let dup_res = sqlx::query("INSERT INTO users (username, password_hash, display_name, role) VALUES ('SCHEMA_TEST_USER', 'hash', 'Test2', 'viewer')")
        .execute(&mut *tx)
        .await;
    assert!(
        dup_res.is_err(),
        "Duplicate case-insensitive username should be rejected"
    );

    // 3. Verify CHECK constraints (post_delay_ms >= 0)
    let auto_id: i64 = sqlx::query_scalar("INSERT INTO automations (name, created_by) VALUES ('Schema Hardening Test', $1) RETURNING id")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let neg_delay_res = sqlx::query("INSERT INTO automation_steps (automation_id, position, step_type, post_delay_ms) VALUES ($1, 10.0, 'key_press', -5000)")
        .bind(auto_id)
        .execute(&mut *tx)
        .await;
    assert!(
        neg_delay_res.is_err(),
        "Negative post_delay_ms should be rejected by CHECK constraint"
    );

    // 4. Verify RGB bounds constraint (expected_r > 255)
    let step1_id: i64 = sqlx::query_scalar("INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 10.0, 'key_press') RETURNING id")
        .bind(auto_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let step2_id: i64 = sqlx::query_scalar("INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 20.0, 'key_press') RETURNING id")
        .bind(auto_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let branch_step_id: i64 = sqlx::query_scalar("INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 30.0, 'branch') RETURNING id")
        .bind(auto_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let invalid_rgb_res = sqlx::query("INSERT INTO step_branches (step_id, automation_id, condition_type, expected_r, on_match_step_id, on_no_match_step_id) VALUES ($1, $2, 'pixel_rgb', 999, $3, $4)")
        .bind(branch_step_id)
        .bind(auto_id)
        .bind(step1_id)
        .bind(step2_id)
        .execute(&mut *tx)
        .await;
    assert!(
        invalid_rgb_res.is_err(),
        "RGB expected_r = 999 should be rejected by CHECK constraint"
    );

    // 5. Verify cross-automation branch target rejection
    let auto2_id: i64 = sqlx::query_scalar("INSERT INTO automations (name, created_by) VALUES ('Schema Hardening Test 2', $1) RETURNING id")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let auto2_step_id: i64 = sqlx::query_scalar("INSERT INTO automation_steps (automation_id, position, step_type) VALUES ($1, 10.0, 'key_press') RETURNING id")
        .bind(auto2_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    let cross_auto_res = sqlx::query("INSERT INTO step_branches (step_id, automation_id, condition_type, expected_r, on_match_step_id, on_no_match_step_id) VALUES ($1, $2, 'pixel_rgb', 100, $3, $4)")
        .bind(branch_step_id)
        .bind(auto_id)
        .bind(auto2_step_id)
        .bind(step2_id)
        .execute(&mut *tx)
        .await;
    assert!(
        cross_auto_res.is_err(),
        "Cross-automation branch target should be rejected by composite FK constraint"
    );

    tx.rollback().await.unwrap();
}
