use aws_sdk_s3::Client;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::HashSet;
use tracing::{error, info, instrument};

/// Prunes expired or old sessions from the database.
#[instrument(skip(pool))]
pub async fn prune_sessions(pool: &PgPool, retention_days: u32) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE expires_at < NOW()
           OR created_at < NOW() - ($1 * INTERVAL '1 day')
        "#,
    )
    .bind(retention_days as i32)
    .execute(pool)
    .await?;

    let rows_affected = result.rows_affected();
    if rows_affected > 0 {
        info!("Pruned {} expired/old session(s)", rows_affected);
    }
    Ok(rows_affected)
}

/// Prunes audit log entries older than `retention_days`.
#[instrument(skip(pool))]
pub async fn prune_audit_logs(pool: &PgPool, retention_days: u32) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM audit_log
        WHERE created_at < NOW() - ($1 * INTERVAL '1 day')
        "#,
    )
    .bind(retention_days as i32)
    .execute(pool)
    .await?;

    let rows_affected = result.rows_affected();
    if rows_affected > 0 {
        info!("Pruned {} old audit log record(s)", rows_affected);
    }
    Ok(rows_affected)
}

/// Prunes task run step logs for completed task runs older than `retention_days`.
#[instrument(skip(pool))]
pub async fn prune_task_run_steps(pool: &PgPool, retention_days: u32) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM task_run_steps
        WHERE task_run_id IN (
            SELECT id FROM task_runs
            WHERE status IN ('succeeded', 'failed', 'cancelled', 'lost')
              AND queued_at < NOW() - ($1 * INTERVAL '1 day')
        )
        "#,
    )
    .bind(retention_days as i32)
    .execute(pool)
    .await?;

    let rows_affected = result.rows_affected();
    if rows_affected > 0 {
        info!("Pruned {} task run step record(s)", rows_affected);
    }
    Ok(rows_affected)
}

struct OldScreenshot {
    id: i64,
    object_storage_key: String,
}

/// Prunes old screenshots from the DB first, then removes associated S3 objects.
#[instrument(skip(pool, s3_client))]
pub async fn prune_old_screenshots(
    pool: &PgPool,
    s3_client: &Client,
    bucket: &str,
    retention_days: u32,
) -> Result<(u64, u64), String> {
    let rows = sqlx::query(
        r#"
        SELECT id, object_storage_key
        FROM step_screenshots
        WHERE created_at < NOW() - ($1 * INTERVAL '1 day')
        "#,
    )
    .bind(retention_days as i32)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query old screenshots: {}", e))?
    .into_iter()
    .map(|row| {
        use sqlx::Row;
        OldScreenshot {
            id: row.get("id"),
            object_storage_key: row.get("object_storage_key"),
        }
    })
    .collect::<Vec<_>>();

    let mut db_deleted = 0u64;
    let mut s3_deleted = 0u64;

    for item in rows {
        // Delete from DB first to ensure data consistency
        let res = sqlx::query("DELETE FROM step_screenshots WHERE id = $1")
            .bind(item.id)
            .execute(pool)
            .await;

        match res {
            Ok(del) if del.rows_affected() > 0 => {
                db_deleted += del.rows_affected();
                // DB record deleted; now delete object from S3
                match s3_client
                    .delete_object()
                    .bucket(bucket)
                    .key(&item.object_storage_key)
                    .send()
                    .await
                {
                    Ok(_) => {
                        s3_deleted += 1;
                    }
                    Err(e) => {
                        error!(
                            "Failed to delete S3 object '{}' for screenshot ID {}: {}",
                            item.object_storage_key, item.id, e
                        );
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                error!("Failed to delete screenshot DB row ID {}: {}", item.id, e);
            }
        }
    }

    if db_deleted > 0 {
        info!(
            "Pruned {} screenshot record(s) from DB and {} object(s) from S3",
            db_deleted, s3_deleted
        );
    }

    Ok((db_deleted, s3_deleted))
}

/// Cleans up abandoned `tmp/` uploads older than `max_age_hours`.
#[instrument(skip(s3_client))]
pub async fn cleanup_abandoned_tmp_uploads(
    s3_client: &Client,
    bucket: &str,
    max_age_hours: u64,
) -> Result<u64, String> {
    let mut count = 0u64;
    let cutoff = Utc::now() - chrono::Duration::hours(max_age_hours as i64);

    let list_res = s3_client
        .list_objects_v2()
        .bucket(bucket)
        .prefix("tmp/")
        .send()
        .await
        .map_err(|e| format!("Failed to list tmp/ objects in S3: {}", e))?;

    if let Some(contents) = list_res.contents {
        for obj in contents {
            if let Some(key) = obj.key {
                let should_delete = match obj.last_modified {
                    Some(lm) => {
                        let sec = lm.as_secs_f64();
                        let obj_time = DateTime::from_timestamp(sec as i64, 0).unwrap_or(Utc::now());
                        obj_time < cutoff
                    }
                    None => true,
                };

                if should_delete {
                    if let Err(e) = s3_client.delete_object().bucket(bucket).key(&key).send().await {
                        error!("Failed to delete abandoned tmp object '{}': {}", key, e);
                    } else {
                        count += 1;
                    }
                }
            }
        }
    }

    if count > 0 {
        info!("Cleaned up {} abandoned tmp upload object(s) from S3", count);
    }

    Ok(count)
}

/// Scans S3 `bitmaps/` and `screenshots/` prefixes and deletes orphan S3 objects not referenced in DB.
#[instrument(skip(pool, s3_client))]
pub async fn gc_orphan_s3_objects(
    pool: &PgPool,
    s3_client: &Client,
    bucket: &str,
) -> Result<u64, String> {
    // 1. Fetch valid keys from DB
    let bitmap_keys: Vec<String> = sqlx::query_scalar("SELECT object_storage_key FROM bitmaps")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to fetch bitmap keys: {}", e))?;

    let screenshot_keys: Vec<String> =
        sqlx::query_scalar("SELECT object_storage_key FROM step_screenshots")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("Failed to fetch screenshot keys: {}", e))?;

    let db_keys: HashSet<String> = bitmap_keys.into_iter().chain(screenshot_keys).collect();

    let mut deleted_count = 0u64;

    for prefix in &["bitmaps/", "screenshots/"] {
        let list_res = s3_client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(*prefix)
            .send()
            .await
            .map_err(|e| format!("Failed to list '{}' objects in S3: {}", prefix, e))?;

        if let Some(contents) = list_res.contents {
            for obj in contents {
                if let Some(key) = obj.key {
                    if !db_keys.contains(&key) {
                        info!("Found orphan S3 object '{}', deleting...", key);
                        if let Err(e) = s3_client
                            .delete_object()
                            .bucket(bucket)
                            .key(&key)
                            .send()
                            .await
                        {
                            error!("Failed to delete orphan S3 object '{}': {}", key, e);
                        } else {
                            deleted_count += 1;
                        }
                    }
                }
            }
        }
    }

    if deleted_count > 0 {
        info!("GC removed {} orphan S3 object(s)", deleted_count);
    }

    Ok(deleted_count)
}

/// Runs all retention and cleanup routines according to app configuration.
#[instrument(skip(pool, s3_client))]
pub async fn run_all_retention_jobs(
    pool: &PgPool,
    s3_client: &Client,
    config: &crate::config::Config,
) {
    if let Err(e) = prune_sessions(pool, config.session_retention_days).await {
        error!("Error pruning sessions: {}", e);
    }
    if let Err(e) = prune_audit_logs(pool, config.audit_log_retention_days).await {
        error!("Error pruning audit logs: {}", e);
    }
    if let Err(e) = prune_task_run_steps(pool, config.task_run_step_retention_days).await {
        error!("Error pruning task run steps: {}", e);
    }
    if let Err(e) = prune_old_screenshots(
        pool,
        s3_client,
        &config.s3_bucket,
        config.screenshot_retention_days,
    )
    .await
    {
        error!("Error pruning old screenshots: {}", e);
    }
    if let Err(e) = cleanup_abandoned_tmp_uploads(s3_client, &config.s3_bucket, 24).await {
        error!("Error cleaning up tmp uploads: {}", e);
    }
    if let Err(e) = gc_orphan_s3_objects(pool, s3_client, &config.s3_bucket).await {
        error!("Error in orphan object GC: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cutoff_calculation() {
        let now = Utc::now();
        let retention_days = 30;
        let cutoff = now - chrono::Duration::days(retention_days as i64);
        assert!(cutoff < now);
    }
}
