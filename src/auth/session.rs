use sqlx::PgPool;
use std::net::IpAddr;
use uuid::Uuid;

#[tracing::instrument(skip(pool))]
pub async fn create_session(
    pool: &PgPool,
    user_id: i64,
    ip_address: Option<IpAddr>,
    user_agent: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let session_id = Uuid::new_v4();
    let ip_str = ip_address.map(|ip| ip.to_string());

    sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, created_at, expires_at, ip_address, user_agent)
        VALUES ($1, $2, now(), now() + interval '1 day', CAST($3 AS inet), $4)
        "#,
    )
    .bind(session_id)
    .bind(user_id)
    .bind(ip_str)
    .bind(user_agent)
    .execute(pool)
    .await?;

    Ok(session_id)
}

#[tracing::instrument(skip(pool))]
pub async fn delete_session(pool: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE id = $1
        "#,
    )
    .bind(session_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub fn create_session_cookie(session_id: Uuid) -> String {
    format!("session_id={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400", session_id)
}

pub fn clear_session_cookie() -> String {
    "session_id=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string()
}

pub fn extract_session_id(cookie_header: &str) -> Option<Uuid> {
    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("session_id=") {
            if let Ok(id) = Uuid::parse_str(value) {
                return Some(id);
            }
        }
    }
    None
}
