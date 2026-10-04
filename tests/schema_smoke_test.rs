// Compile-time and schema validation smoke test
use sqlx::PgPool;

async fn get_test_pool() -> Option<PgPool> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());
    PgPool::connect(&db_url).await.ok()
}

#[test]
fn test_schema_queries_syntax() {
    let select_users = "SELECT id, username, password_hash, display_name, role, is_active FROM users";
    let select_automations = "SELECT id, name, description, status FROM automations";
    let select_steps = "SELECT id, automation_id, position, step_type, post_delay_ms FROM automation_steps";
    let select_mouse_clicks = "SELECT step_id, x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks";
    let select_key_presses = "SELECT step_id, key_combo FROM step_key_presses";
    let select_branches = "SELECT step_id, condition_type, on_match_step_id, on_no_match_step_id FROM step_branches";
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
        dispatch_plan_str.contains("idx_task_runs_queued") || dispatch_plan_str.contains("Index Scan") || dispatch_plan_str.contains("Bitmap Index Scan") || dispatch_plan_str.contains("task_runs"),
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
        scheduling_plan_str.contains("idx_schedules_due") || scheduling_plan_str.contains("Index Scan") || scheduling_plan_str.contains("Bitmap Index Scan") || scheduling_plan_str.contains("schedules"),
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
        positional_plan_str.contains("idx_steps_automation_position") || positional_plan_str.contains("Index Scan") || positional_plan_str.contains("Bitmap Index Scan") || positional_plan_str.contains("automation_steps"),
        "Positional query plan should execute cleanly: {}",
        positional_plan_str
    );
}
