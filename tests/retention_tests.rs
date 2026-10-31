#![allow(clippy::unwrap_used)]
use deskdispatch::retention::*;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::time::Duration;

async fn get_test_pool() -> Option<sqlx::PgPool> {
    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_millis(500))
        .connect(&db_url)
        .await
        .ok()
}

struct RetentionCleanupGuard {
    pool: sqlx::PgPool,
    user_ids: Vec<i64>,
    session_ids: Vec<uuid::Uuid>,
    audit_ids: Vec<i64>,
    auto_ids: Vec<i64>,
    task_run_ids: Vec<i64>,
}

impl RetentionCleanupGuard {
    fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            user_ids: Vec::new(),
            session_ids: Vec::new(),
            audit_ids: Vec::new(),
            auto_ids: Vec::new(),
            task_run_ids: Vec::new(),
        }
    }
}

impl Drop for RetentionCleanupGuard {
    fn drop(&mut self) {
        let options = self.pool.connect_options().clone();
        let user_ids = self.user_ids.clone();
        let session_ids = self.session_ids.clone();
        let audit_ids = self.audit_ids.clone();
        let auto_ids = self.auto_ids.clone();
        let task_run_ids = self.task_run_ids.clone();

        let cleanup = async move {
            let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(std::time::Duration::from_secs(2))
                .connect_with((*options).clone())
                .await
            else {
                return;
            };
            if !session_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM sessions WHERE id = ANY($1)")
                    .bind(&session_ids)
                    .execute(&pool)
                    .await;
            }
            if !audit_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM audit_log WHERE id = ANY($1)")
                    .bind(&audit_ids)
                    .execute(&pool)
                    .await;
            }
            if !task_run_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM task_run_steps WHERE task_run_id = ANY($1)")
                    .bind(&task_run_ids)
                    .execute(&pool)
                    .await;
                let _ = sqlx::query("DELETE FROM task_runs WHERE id = ANY($1)")
                    .bind(&task_run_ids)
                    .execute(&pool)
                    .await;
            }
            if !auto_ids.is_empty() {
                let _ = sqlx::query("DELETE FROM automation_steps WHERE automation_id = ANY($1)")
                    .bind(&auto_ids)
                    .execute(&pool)
                    .await;
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
                rt.shutdown_timeout(std::time::Duration::from_millis(500));
            }
        })
        .join();
    }
}

#[tokio::test]
async fn test_prune_sessions_and_audit_logs() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_prune_sessions_and_audit_logs");
        return;
    };

    let mut guard = RetentionCleanupGuard::new(pool.clone());

    // Insert unique test user
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let user_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO users (username, password_hash, display_name, role)
        VALUES ($1, 'hash', 'Retention User', 'admin')
        RETURNING id
        "#,
    )
    .bind(&username)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert test user");

    guard.user_ids.push(user_id);

    // 1. Insert expired session and valid session
    let expired_session_id = uuid::Uuid::new_v4();
    let valid_session_id = uuid::Uuid::new_v4();
    guard.session_ids.push(expired_session_id);
    guard.session_ids.push(valid_session_id);

    sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, expires_at, created_at, last_active_at)
        VALUES ($1, $2, NOW() - INTERVAL '1 hour', NOW() - INTERVAL '10 days', NOW() - INTERVAL '1 hour')
        "#,
    )
    .bind(expired_session_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, expires_at, created_at, last_active_at)
        VALUES ($1, $2, NOW() + INTERVAL '1 day', NOW(), NOW())
        "#,
    )
    .bind(valid_session_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    // Prune sessions
    let pruned_sessions = prune_sessions(&pool, 7)
        .await
        .expect("prune_sessions failed");
    assert!(
        pruned_sessions >= 1,
        "At least expired session should be pruned"
    );

    // Verify expired session is gone, valid session exists
    let expired_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(expired_session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!expired_exists, "Expired session must be pruned");

    let valid_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(valid_session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(valid_exists, "Valid session must be retained");

    // 2. Insert old and recent audit logs
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

    guard.audit_ids.push(old_audit_id);
    guard.audit_ids.push(recent_audit_id);

    let pruned_audits = prune_audit_logs(&pool, 90)
        .await
        .expect("prune_audit_logs failed");
    assert!(
        pruned_audits >= 1,
        "At least old audit log should be pruned"
    );

    let old_audit_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
            .bind(old_audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!old_audit_exists, "Old audit log must be pruned");

    let recent_audit_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE id = $1)")
            .bind(recent_audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(recent_audit_exists, "Recent audit log must be retained");
}

#[tokio::test]
async fn test_prune_task_run_steps() {
    let Some(pool) = get_test_pool().await else {
        println!("Database not available, skipping test_prune_task_run_steps");
        return;
    };

    let mut guard = RetentionCleanupGuard::new(pool.clone());

    // Insert dummy automation step and automation
    let auto_id: i64 = sqlx::query_scalar(
        "INSERT INTO automations (name, created_at) VALUES ('Retention Test Auto', NOW()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to insert test automation");

    guard.auto_ids.push(auto_id);

    let step_id: i64 = sqlx::query_scalar(
        "INSERT INTO automation_steps (automation_id, position, step_type, label) VALUES ($1, 10.0, 'key_press', 'Step 1') RETURNING id",
    )
    .bind(auto_id)
    .fetch_one(&pool)
    .await
    .unwrap();

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

    guard.task_run_ids.push(old_run_id);
    guard.task_run_ids.push(recent_run_id);

    // Insert step execution logs into task_run_steps matching real schema
    sqlx::query(
        r#"
        INSERT INTO task_run_steps (task_run_id, step_id, seq, result, started_at, completed_at)
        VALUES ($1, $2, 1, 'success', NOW() - INTERVAL '40 days', NOW() - INTERVAL '40 days')
        "#,
    )
    .bind(old_run_id)
    .bind(step_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO task_run_steps (task_run_id, step_id, seq, result, started_at, completed_at)
        VALUES ($1, $2, 1, 'success', NOW(), NOW())
        "#,
    )
    .bind(recent_run_id)
    .bind(step_id)
    .execute(&pool)
    .await
    .unwrap();

    // Prune task run steps (>30 days)
    let pruned = prune_task_run_steps(&pool, 30)
        .await
        .expect("prune_task_run_steps failed");
    assert!(
        pruned >= 1,
        "At least 1 step from old task run should be pruned"
    );

    let old_steps_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_run_steps WHERE task_run_id = $1")
            .bind(old_run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        old_steps_count, 0,
        "Steps for task run older than 30 days should be deleted"
    );

    let recent_steps_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_run_steps WHERE task_run_id = $1")
            .bind(recent_run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        recent_steps_count, 1,
        "Steps for recent task run should be retained"
    );

    // Verify task_runs records themselves were retained
    let old_run_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_runs WHERE id = $1)")
            .bind(old_run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        old_run_exists,
        "Task run record itself should remain after step pruning"
    );
}
