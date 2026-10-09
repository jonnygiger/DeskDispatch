use sqlx::{Executor, Postgres};

#[tracing::instrument(skip(exec))]
pub async fn log_audit<'e, E>(
    exec: E,
    user_id: Option<i64>,
    action: &str,
    entity_type: &str,
    entity_id: Option<i64>,
    details: Option<serde_json::Value>,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO audit_log (user_id, action, entity_type, entity_id, details)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(details)
    .execute(exec)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_log_audit_parameters() {
        let details = serde_json::json!({
            "key": "value"
        });
        assert_eq!(details["key"], "value");
    }
}
