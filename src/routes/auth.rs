use askama::Template;
use axum::{
    extract::{ConnectInfo, Form, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::Row;
use std::net::{IpAddr, SocketAddr};

use crate::auth::{
    clear_session_cookie, create_session, create_session_cookie, delete_session, log_audit,
    session::extract_session_id, verify_password_async, OptionalAuthUser,
};
use axum::routing::{get, post};
use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/logout", post(post_logout_handler))
}

pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => {
                tracing::error!("Template rendering error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to render template",
                )
                    .into_response()
            }
        }
    }
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

const DUMMY_ARGON2_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$cG03OThscDRvYm9oMDAwMA$R3841R3841R3841R3841R3841R3841R3841R3841";

struct UserRow {
    id: i64,
    password_hash: String,
    is_active: bool,
}

#[tracing::instrument]
pub async fn get_login_handler() -> impl IntoResponse {
    HtmlTemplate(LoginTemplate { error: None })
}

#[tracing::instrument(skip(headers, state, form))]
pub async fn post_login_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let client_ip = extract_client_ip(
        &headers,
        Some(&addr),
        state.config.trust_proxy_headers,
    );
    let username_trim = form.username.trim();

    if let Err(rate_err) = state.rate_limiter.check_rate_limit(client_ip, username_trim) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            HtmlTemplate(LoginTemplate {
                error: Some(rate_err),
            }),
        )
            .into_response();
    }

    let user_res = sqlx::query(
        r#"
        SELECT id, password_hash, is_active
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(username_trim)
    .fetch_optional(&state.db)
    .await;

    let user_opt = match user_res {
        Ok(Some(row)) => {
            let u = UserRow {
                id: row.get("id"),
                password_hash: row.get("password_hash"),
                is_active: row.get("is_active"),
            };
            if u.is_active {
                Some(u)
            } else {
                None
            }
        }
        Ok(None) => None,
        Err(e) => {
            tracing::error!("Database error during login: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(LoginTemplate {
                    error: Some("Internal server error".to_string()),
                }),
            )
                .into_response();
        }
    };

    let (target_hash, real_user) = match user_opt {
        Some(u) => (u.password_hash.clone(), Some(u)),
        None => (DUMMY_ARGON2_HASH.to_string(), None),
    };

    let is_valid = match verify_password_async(
        state.rate_limiter.argon2_semaphore.clone(),
        form.password,
        target_hash,
    )
    .await
    {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!("Password verification error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(LoginTemplate {
                    error: Some("Internal server error".to_string()),
                }),
            )
                .into_response();
        }
    };

    let user = match (is_valid, real_user) {
        (true, Some(u)) => u,
        _ => {
            state.rate_limiter.record_failure(client_ip, username_trim);
            return (
                StatusCode::UNAUTHORIZED,
                HtmlTemplate(LoginTemplate {
                    error: Some("Invalid username or password".to_string()),
                }),
            )
                .into_response();
        }
    };

    state.rate_limiter.clear(client_ip, username_trim);

    if let Err(e) = sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        tracing::error!("Failed to update last_login_at: {}", e);
    }

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|h| h.to_str().ok());

    let session_id = match create_session(&state.db, user.id, Some(client_ip), user_agent).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to create session: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(LoginTemplate {
                    error: Some("Failed to create session".to_string()),
                }),
            )
                .into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "login",
        "user",
        Some(user.id),
        None,
    )
    .await;

    let is_secure = state.config.is_production() || state.config.public_base_url.starts_with("https://");
    let cookie = create_session_cookie(session_id, is_secure);
    (
        StatusCode::SEE_OTHER,
        [(header::SET_COOKIE, cookie), (header::LOCATION, "/".to_string())],
    )
        .into_response()
}

#[tracing::instrument(skip(state, user, headers))]
pub async fn post_logout_handler(
    State(state): State<AppState>,
    user: OptionalAuthUser,
    headers: HeaderMap,
) -> Response {
    if let Some(u) = &user.0 {
        let _ = log_audit(&state.db, Some(u.id), "logout", "user", Some(u.id), None).await;
    }

    if let Some(cookie_header) = headers.get(header::COOKIE).and_then(|h| h.to_str().ok()) {
        if let Some(session_id) = extract_session_id(cookie_header) {
            let _ = delete_session(&state.db, session_id).await;
        }
    }

    let cookie = clear_session_cookie();
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, "/login".to_string()),
        ],
    )
        .into_response()
}

pub fn extract_client_ip(
    headers: &HeaderMap,
    connect_info: Option<&SocketAddr>,
    trust_proxy_headers: bool,
) -> IpAddr {
    if trust_proxy_headers {
        if let Some(forwarded) = headers.get("X-Forwarded-For").and_then(|h| h.to_str().ok()) {
            if let Some(first_ip) = forwarded.split(',').next() {
                if let Ok(ip) = first_ip.trim().parse() {
                    return ip;
                }
            }
        }

        if let Some(real_ip) = headers.get("X-Real-IP").and_then(|h| h.to_str().ok()) {
            if let Ok(ip) = real_ip.trim().parse() {
                return ip;
            }
        }
    }

    if let Some(addr) = connect_info {
        return addr.ip();
    }

    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::IpAddr;

    #[test]
    fn test_extract_client_ip_headers() {
        let conn_addr: SocketAddr = "10.0.0.5:12345".parse().unwrap();

        // When trust_proxy_headers is false, headers are ignored
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", HeaderValue::from_static("203.0.113.195"));
        let ip = extract_client_ip(&headers, Some(&conn_addr), false);
        assert_eq!(ip, conn_addr.ip());

        // When trust_proxy_headers is true:
        // 1. Single IPv4 address in X-Forwarded-For
        let ip = extract_client_ip(&headers, Some(&conn_addr), true);
        assert_eq!(ip, "203.0.113.195".parse::<IpAddr>().unwrap());

        // 2. Multiple comma-separated IPs in X-Forwarded-For (client IP is first)
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Forwarded-For",
            HeaderValue::from_static("198.51.100.1, 203.0.113.195, 70.41.3.18"),
        );
        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip, "198.51.100.1".parse::<IpAddr>().unwrap());

        // 3. X-Forwarded-For takes precedence over X-Real-IP
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", HeaderValue::from_static("203.0.113.195"));
        headers.insert("X-Real-IP", HeaderValue::from_static("198.51.100.22"));
        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip, "203.0.113.195".parse::<IpAddr>().unwrap());

        // 4. X-Real-IP fallback when X-Forwarded-For is missing
        let mut headers = HeaderMap::new();
        headers.insert("X-Real-IP", HeaderValue::from_static("198.51.100.22"));
        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip, "198.51.100.22".parse::<IpAddr>().unwrap());

        // 5. IPv6 address parsing in X-Forwarded-For
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", HeaderValue::from_static("2001:db8::1"));
        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip, "2001:db8::1".parse::<IpAddr>().unwrap());

        // 6. Invalid IP format in X-Forwarded-For falls back to valid X-Real-IP
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", HeaderValue::from_static("invalid_ip"));
        headers.insert("X-Real-IP", HeaderValue::from_static("198.51.100.22"));
        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip, "198.51.100.22".parse::<IpAddr>().unwrap());

        // 7. No headers present falls back to connect_info or 127.0.0.1
        let headers = HeaderMap::new();
        let ip = extract_client_ip(&headers, None, false);
        assert_eq!(ip, "127.0.0.1".parse::<IpAddr>().unwrap());
    }
}
