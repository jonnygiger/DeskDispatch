use axum::{
    extract::{FromRef, FromRequest, FromRequestParts, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

use super::user::AuthUser;
use crate::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_csrf_token(session_id: Uuid, session_secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(session_secret.as_bytes())
        .expect("HMAC can take key of any size");
    let mut buf = [0u8; 36];
    let str_uuid = session_id.hyphenated().encode_lower(&mut buf);
    mac.update(str_uuid.as_bytes());
    let result = mac.finalize();
    crate::storage::encode_hex(&result.into_bytes())
}

pub fn validate_csrf_token(token: &str, expected_token: &str) -> bool {
    let a = token.as_bytes();
    let b = expected_token.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[derive(Debug, Clone)]
pub struct CsrfForm<T>(pub T);

impl<S, T> FromRequest<S> for CsrfForm<T>
where
    S: Send + Sync,
    AppState: FromRef<S>,
    T: serde::de::DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request(
        mut req: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let user = match req.extensions().get::<AuthUser>().cloned() {
            Some(u) => u,
            None => {
                let (mut parts, body) = req.into_parts();
                let user_res = AuthUser::from_request_parts(&mut parts, &app_state).await;
                req = axum::extract::Request::from_parts(parts, body);
                match user_res {
                    Ok(u) => u,
                    Err(rejection) => return Err(rejection),
                }
            }
        };

        let (parts, body) = req.into_parts();
        let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => {
                return Err(
                    (StatusCode::BAD_REQUEST, "Failed to read request body").into_response()
                );
            }
        };

        #[derive(serde::Deserialize)]
        struct CsrfExtractor<'a> {
            #[serde(borrow)]
            csrf_token: Option<std::borrow::Cow<'a, str>>,
        }

        let header_csrf = parts
            .headers
            .get("X-CSRF-Token")
            .and_then(|h| h.to_str().ok());

        let body_csrf = if header_csrf.is_none() {
            serde_urlencoded::from_bytes::<CsrfExtractor>(&bytes)
                .ok()
                .and_then(|ext| ext.csrf_token)
        } else {
            None
        };

        let csrf = match (header_csrf, &body_csrf) {
            (Some(c), _) => c,
            (None, Some(c)) => c.as_ref(),
            (None, None) => {
                return Err((StatusCode::FORBIDDEN, "Missing CSRF token").into_response());
            }
        };

        if !validate_csrf_token(csrf, &user.csrf_token) {
            return Err((StatusCode::FORBIDDEN, "Invalid CSRF token").into_response());
        }

        let value: T = match serde_urlencoded::from_bytes(&bytes) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to deserialize CsrfForm payload: {}", e);
                return Err(
                    (StatusCode::BAD_REQUEST, format!("Invalid form data: {}", e)).into_response(),
                );
            }
        };

        Ok(CsrfForm(value))
    }
}

pub async fn csrf_origin_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || method == axum::http::Method::OPTIONS
    {
        return Ok(next.run(req).await);
    }

    if path == "/login" || path.starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    if let Some(sec_fetch_site) = req
        .headers()
        .get("Sec-Fetch-Site")
        .and_then(|h| h.to_str().ok())
        && sec_fetch_site == "cross-site"
    {
        tracing::warn!(
            path = path,
            "Rejected cross-site request via Sec-Fetch-Site"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(origin_header) = req
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|h| h.to_str().ok())
    {
        let expected_base = &state.config.public_base_url;
        let expected_origin = expected_base.trim_end_matches('/');

        let origin_matches = if origin_header == expected_origin {
            true
        } else if let Some(host_header) = req
            .headers()
            .get(axum::http::header::HOST)
            .and_then(|h| h.to_str().ok())
        {
            origin_header.rsplit("://").next().unwrap_or("") == host_header
        } else {
            false
        };

        if !origin_matches {
            tracing::warn!(
                origin = origin_header,
                expected = expected_origin,
                path = path,
                "CSRF Origin header mismatch"
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }

    Ok(next.run(req).await)
}

pub use csrf_origin_middleware as csrf_middleware;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_csrf_validation() {
        let sid = Uuid::new_v4();
        let secret = "test_session_secret_12345";
        let token = generate_csrf_token(sid, secret);

        assert!(validate_csrf_token(&token, &token));

        let wrong_token = generate_csrf_token(Uuid::new_v4(), secret);
        assert!(!validate_csrf_token(&token, &wrong_token));

        assert!(!validate_csrf_token(&token, "short_invalid"));
    }
}
