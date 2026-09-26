use axum::{
    extract::State,
    http::StatusCode,
    response::Response,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_csrf_token(session_id: Uuid, session_secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(session_secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(session_id.to_string().as_bytes());
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

pub fn validate_csrf_token(token: &str, expected_token: &str) -> bool {
    token == expected_token
}

pub async fn csrf_middleware(
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

    // Exempt endpoints: /login, worker API /api/*
    if path == "/login" || path.starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    let cookie_header = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|h| h.to_str().ok());

    let session_id = cookie_header
        .and_then(super::session::extract_session_id)
        .ok_or(StatusCode::FORBIDDEN)?;

    let expected_csrf = generate_csrf_token(session_id, &state.config.session_secret);

    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut provided_csrf: Option<String> = None;

    if let Some(header_val) = parts.headers.get("X-CSRF-Token").and_then(|h| h.to_str().ok()) {
        provided_csrf = Some(header_val.to_string());
    }

    if provided_csrf.is_none() {
        if let Ok(params) =
            serde_urlencoded::from_bytes::<std::collections::HashMap<String, String>>(&bytes)
        {
            provided_csrf = params.get("csrf_token").cloned();
        }
    }

    if let Some(csrf) = provided_csrf {
        if validate_csrf_token(&csrf, &expected_csrf) {
            let req = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
            return Ok(next.run(req).await);
        }
    }

    Err(StatusCode::FORBIDDEN)
}
