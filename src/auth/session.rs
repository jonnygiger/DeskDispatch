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
        INSERT INTO sessions (id, user_id, created_at, expires_at, last_active_at, ip_address, user_agent)
        VALUES ($1, $2, now(), now() + interval '7 days', now(), CAST($3 AS inet), $4)
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

#[tracing::instrument(skip(pool))]
pub async fn purge_expired_sessions(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE expires_at <= now() OR last_active_at <= now() - interval '2 hours'
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

#[tracing::instrument(skip(pool))]
pub async fn revoke_user_sessions_except(
    pool: &PgPool,
    user_id: i64,
    current_session_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE user_id = $1 AND id != $2
        "#,
    )
    .bind(user_id)
    .bind(current_session_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub fn create_session_cookie(session_id: Uuid, secure: bool) -> String {
    if secure {
        format!(
            "__Host-session_id={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=604800",
            session_id
        )
    } else {
        format!(
            "session_id={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800",
            session_id
        )
    }
}

pub fn clear_session_cookie() -> String {
    "__Host-session_id=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string()
}

pub fn extract_session_id(cookie_header: &str) -> Option<Uuid> {
    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("__Host-session_id=")
            && let Ok(id) = Uuid::parse_str(value)
        {
            return Some(id);
        }
        if let Some(value) = cookie.strip_prefix("session_id=")
            && let Ok(id) = Uuid::parse_str(value)
        {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_cookies_and_extraction() {
        let sid = Uuid::new_v4();
        let secure_cookie = create_session_cookie(sid, true);
        assert!(secure_cookie.contains("__Host-session_id="));
        assert!(secure_cookie.contains("Secure"));

        let insecure_cookie = create_session_cookie(sid, false);
        assert!(insecure_cookie.contains("session_id="));
        assert!(!insecure_cookie.contains("__Host-"));

        assert_eq!(extract_session_id(&secure_cookie), Some(sid));
        assert_eq!(extract_session_id(&insecure_cookie), Some(sid));
    }

    #[test]
    fn test_extract_session_id_edge_cases_and_clear_cookie() {
        // Arrange
        let valid_uuid_1 = Uuid::new_v4();
        let valid_uuid_2 = Uuid::new_v4();

        // 1. Verify clear_session_cookie exact attributes
        let clear_cookie = clear_session_cookie();
        assert!(clear_cookie.contains("__Host-session_id="));
        assert!(clear_cookie.contains("Max-Age=0"));
        assert!(clear_cookie.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(clear_cookie.contains("Path=/"));
        assert!(clear_cookie.contains("HttpOnly"));
        assert!(clear_cookie.contains("Secure"));
        assert!(clear_cookie.contains("SameSite=Lax"));

        // 2. Extract session ID from multi-cookie header with leading/trailing whitespace
        let multi_cookie = format!(
            "  theme=dark;   __Host-session_id={} ;   lang=en ",
            valid_uuid_1
        );
        assert_eq!(extract_session_id(&multi_cookie), Some(valid_uuid_1));

        // 3. Extract session ID from header without space after semicolon
        let no_space_cookie = format!("a=1;session_id={};b=2", valid_uuid_2);
        assert_eq!(extract_session_id(&no_space_cookie), Some(valid_uuid_2));

        // 4. Invalid non-UUID cookie values return None
        assert_eq!(extract_session_id("session_id=not-a-valid-uuid"), None);
        assert_eq!(extract_session_id("__Host-session_id=12345"), None);

        // 5. Fallback: invalid __Host-session_id followed by valid session_id
        let fallback_cookie = format!("__Host-session_id=invalid-uuid; session_id={}", valid_uuid_1);
        assert_eq!(extract_session_id(&fallback_cookie), Some(valid_uuid_1));

        // 6. Precedence: valid __Host-session_id encountered first takes precedence
        let precedence_cookie = format!(
            "__Host-session_id={}; session_id={}",
            valid_uuid_1, valid_uuid_2
        );
        assert_eq!(extract_session_id(&precedence_cookie), Some(valid_uuid_1));

        // 7. Non-matching key name prefixes return None
        let false_prefix_cookie = format!(
            "not_session_id={}; my__Host-session_id={}",
            valid_uuid_1, valid_uuid_2
        );
        assert_eq!(extract_session_id(&false_prefix_cookie), None);

        // 8. Empty or whitespace-only header returns None
        assert_eq!(extract_session_id(""), None);
        assert_eq!(extract_session_id("   \t  \n "), None);
    }
}
