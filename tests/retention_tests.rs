use deskdispatch::retention::*;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::time::Duration;

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = env::var("DATABASE_URL").ok().filter(|v| !v.trim().is_empty())?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&db_url)
        .await
        .ok()?;

    let _ = sqlx::migrate!("./migrations").run(&pool).await;
    Some(pool)
}

#[tokio::test]
async fn test_prune_sessions_and_audit_logs() {
    let pool = match get_test_pool().await {
        Some(p) => p,
        None => {
            eprintln!("Skipping test_prune_sessions_and_audit_logs: DATABASE_URL not set");
            return;
        }
    };

    // Insert test user if needed
    let user_id: i64 = match sqlx::query_scalar(
        r#"
        INSERT INTO users (username, password_hash, display_name, role)
        VALUES ('retention_test_user', 'hash', 'Retention User', 'admin')
        ON CONFLICT (username) DO UPDATE SET display_name = EXCLUDED.display_name
        RETURNING id
        "#,
    )
    .fetch_one(&pool)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Failed to insert test user: {}", e);
            return;
        }
    };

    // 1. Insert expired session and valid session
    let expired_session_id = uuid::Uuid::new_v4();
    let valid_session_id = uuid::Uuid::new_v4();

    let _ = sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, expires_at, created_at, last_active_at)
        VALUES ($1, $2, NOW() - INTERVAL '1 hour', NOW() - INTERVAL '10 days', NOW() - INTERVAL '1 hour')
        "#,
    )
    .bind(expired_session_id)
    .bind(user_id)
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, expires_at, created_at, last_active_at)
        VALUES ($1, $2, NOW() + INTERVAL '1 day', NOW(), NOW())
        "#,
    )
    .bind(valid_session_id)
    .bind(user_id)
    .execute(&pool)
    .await;

    // Prune sessions
    let pruned_sessions = prune_sessions(&pool, 7).await.expect("prune_sessions failed");
    assert!(pruned_sessions >= 1);

    // Verify expired session is gone, valid session exists
    let expired_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
        .bind(expired_session_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!expired_exists);

    let valid_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
        .bind(valid_session_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(valid_exists);

    // Cleanup session
    let _ = sqlx::query("DELETE FROM sessions WHERE id = $1").bind(valid_session_id).execute(&pool).await;

    // 2. Insert old and recent audit log
    let old_audit_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_log (user_id, action, entity_type, entity_id, created_at)
        VALUES ($1, 'test_old', 'user', '1', NOW() - INTERVAL '100 days')
        RETURNING id
        "#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let recent_audit_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_log (user_id, action, entity_type, entity_id, created_at)
        VALUES ($1, 'test_recent', 'user', '1', NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let pruned_audits = prune_audit_logs(&pool, 90).await.expect("prune_audit_logs failed");
    assert!(pruned_audits >= 1);

    let old_audit_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
        .bind(old_audit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!old_audit_exists);

    let recent_audit_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
        .bind(recent_audit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(recent_audit_exists);

    // Cleanup audit log and user
    let _ = sqlx::query("DELETE FROM audit_log WHERE id = $1").bind(recent_audit_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await;
}

#[tokio::test]
async fn test_prune_task_run_steps() {
    let pool = match get_test_pool().await {
        Some(p) => p,
        None => {
            eprintln!("Skipping test_prune_task_run_steps: DATABASE_URL not set");
            return;
        }
    };

    // Insert dummy automation
    let auto_id: i64 = match sqlx::query_scalar(
        "INSERT INTO automations (name, created_at) VALUES ('Retention Test Auto', NOW()) RETURNING id",
    )
    .fetch_one(&pool)
    .await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Failed to insert automation: {}", e);
            return;
        }
    };

    // Insert old completed task run and recent completed task run
    let old_run_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_runs (automation_id, status, queued_at)
        VALUES ($1, 'succeeded', NOW() - INTERVAL '40 days')
        RETURNING id
        "#,
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let recent_run_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO task_runs (automation_id, status, queued_at)
        VALUES ($1, 'succeeded', NOW())
        RETURNING id
        "#,
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Insert steps into task_run_steps
    let _ = sqlx::query(
        "INSERT INTO task_run_steps (task_run_id, step_name, status) VALUES ($1, 'Old Step', 'succeeded')",
    )
    .bind(old_run_id)
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO task_run_steps (task_run_id, step_name, status) VALUES ($1, 'Recent Step', 'succeeded')",
    )
    .bind(recent_run_id)
    .execute(&pool)
    .await;

    // Prune task run steps (>30 days)
    let pruned = prune_task_run_steps(&pool, 30).await.expect("prune_task_run_steps failed");
    assert!(pruned >= 1);

    let old_steps_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_run_steps WHERE task_run_id = $1")
        .bind(old_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_steps_count, 0);

    let recent_steps_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_run_steps WHERE task_run_id = $1")
        .bind(recent_run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recent_steps_count, 1);

    // Cleanup
    let _ = sqlx::query("DELETE FROM task_run_steps WHERE task_run_id = $1").bind(recent_run_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM task_runs WHERE automation_id = $1").bind(auto_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM automations WHERE id = $1").bind(auto_id).execute(&pool).await;
}
