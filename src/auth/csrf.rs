use axum::{
    extract::{FromRef, FromRequest, FromRequestParts, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use super::user::AuthUser;
use crate::AppState;

pub fn generate_csrf_token(session_id: Uuid, session_secret: &str) -> String {
    let mut buf = [0u8; 36];
    let str_uuid = session_id.hyphenated().encode_lower(&mut buf);
    let mac_result = crate::storage::hmac_sha256(session_secret.as_bytes(), str_uuid.as_bytes());
    crate::storage::encode_hex(&mac_result)
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

        let header_csrf = parts
            .headers
            .get("X-CSRF-Token")
            .and_then(|h| h.to_str().ok());

        let body_csrf = if header_csrf.is_none() {
            url::form_urlencoded::parse(&bytes)
                .find(|(k, _)| k == "csrf_token")
                .map(|(_, v)| v)
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

        let req = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
        let value: T = match axum::Form::<T>::from_request(req, state).await {
            Ok(axum::Form(v)) => v,
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
    let method = req.method();
    let path = req.uri().path();

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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::{get, post},
    };
    use secrecy::SecretString;
    use tower::ServiceExt;

    fn create_test_state() -> AppState {
        let config = crate::config::Config {
            database_url: "postgres://postgres:postgres@localhost/deskdispatch".to_string(),
            public_base_url: "http://localhost:3000".to_string(),
            s3_endpoint: None,
            s3_public_endpoint: None,
            s3_bucket: "test-bucket".to_string(),
            s3_access_key: "key".to_string(),
            s3_secret_key: SecretString::from("secret".to_string()),
            s3_region: "us-east-1".to_string(),
            session_secret: SecretString::from("secret".to_string()),
            bind_address: "127.0.0.1:3000".to_string(),
            environment: "development".to_string(),
            min_agent_version: None,
            worker_poll_interval_secs: 5,
            worker_heartbeat_interval_secs: 15,
            trust_proxy_headers: false,
            database_max_connections: 20,
            database_acquire_timeout_secs: 5,
            worker_long_poll_timeout_secs: 10,
            session_retention_days: 7,
            audit_log_retention_days: 90,
            task_run_step_retention_days: 30,
            screenshot_retention_days: 30,
            metrics_bind_address: None,
            unattended_registration_psk: SecretString::from(
                "deskdispatch_wps_psk_secret".to_string(),
            ),
        };

        let pool =
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@localhost/deskdispatch")
                .unwrap();
        let storage = crate::storage::StorageService::new(&config.s3_bucket);
        let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

        AppState {
            db: pool,
            storage,
            config,
            rate_limiter: crate::auth::LoginRateLimiter::default(),
            task_queue_notifier: tx_notify,
            wps_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn build_test_app(state: AppState) -> Router {
        Router::new()
            .route("/test", get(|| async { "OK" }).post(|| async { "OK" }))
            .route("/login", post(|| async { "OK" }))
            .route("/api/v1/workers", post(|| async { "OK" }))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                csrf_origin_middleware,
            ))
            .with_state(state)
    }

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

    #[tokio::test]
    async fn test_csrf_origin_middleware_safe_methods() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Origin", "http://evil.com")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_exempt_paths() {
        let state = create_test_state();

        for path in ["/login", "/api/v1/workers"] {
            let app = build_test_app(state.clone());
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("Sec-Fetch-Site", "cross-site")
                .header("Origin", "http://evil.com")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_sec_fetch_site_cross_site_rejected() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_matching_origin() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_fallback_host_matching() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Origin", "http://app.internal:8080")
            .header("Host", "app.internal:8080")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_mismatched_origin_rejected() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Origin", "http://evil.com")
            .header("Host", "localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_origin_middleware_no_origin_header() {
        let state = create_test_state();
        let app = build_test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
