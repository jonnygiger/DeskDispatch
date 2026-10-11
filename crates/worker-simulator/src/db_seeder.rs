use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Automatically seeds `count` worker registration tokens in PostgreSQL database for instant testing.
pub async fn seed_workers_in_db(
    database_url: &str,
    count: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let pool = PgPool::connect(database_url).await?;

    let mut tokens = Vec::with_capacity(count);

    for i in 1..=count {
        let raw_token = format!("dd_reg_sim_{}_{}", i, Uuid::new_v4());
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        let token_hash_bytes = hasher.finalize().to_vec();

        let hostname = format!("sim-worker-{:03}.local", i);
        let display_name = format!("Simulated Worker {:03}", i);

        // Delete existing worker node with same hostname if re-running
        let _ = sqlx::query("DELETE FROM task_worker_pcs WHERE hostname = $1")
            .bind(&hostname)
            .execute(&pool)
            .await;

        sqlx::query(
            r#"
            INSERT INTO task_worker_pcs (
                hostname, display_name, registration_token_hash,
                registration_token_expires_at, status
            )
            VALUES ($1, $2, $3, now() + INTERVAL '24 hours', 'offline')
            "#,
        )
        .bind(&hostname)
        .bind(&display_name)
        .bind(&token_hash_bytes)
        .execute(&pool)
        .await?;

        tokens.push(raw_token);
    }

    Ok(tokens)
}
