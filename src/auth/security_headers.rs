use axum::{
    extract::State,
    http::header,
    middleware::Next,
    response::Response,
};
use crate::AppState;

/// Middleware to attach security hardening headers to all HTTP responses.
pub async fn security_headers_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    let mut s3_origins = Vec::new();
    if let Some(endpoint) = &state.config.s3_public_endpoint {
        if !endpoint.trim().is_empty() {
            s3_origins.push(endpoint.trim());
        }
    } else if let Some(endpoint) = &state.config.s3_endpoint {
        if !endpoint.trim().is_empty() {
            s3_origins.push(endpoint.trim());
        }
    }

    let extra_origins = if s3_origins.is_empty() {
        String::new()
    } else {
        format!(" {}", s3_origins.join(" "))
    };

    let csp = format!(
        "default-src 'self'; script-src 'none'; frame-ancestors 'none'; form-action 'self'{}; img-src 'self' data:{}; style-src 'self' 'unsafe-inline';",
        extra_origins, extra_origins
    );

    if let Ok(val) = header::HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, val);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        header::HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use crate::auth::LoginRateLimiter;
    use crate::config::Config;
    use secrecy::SecretString;
    use tower::ServiceExt;

    fn create_test_state(s3_public_endpoint: Option<String>) -> AppState {
        let config = Config {
            database_url: "postgres://postgres:postgres@localhost/deskdispatch".to_string(),
            public_base_url: "http://localhost:3000".to_string(),
            s3_endpoint: None,
            s3_public_endpoint,
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
        };

        let pool = sqlx::PgPool::connect_lazy("postgres://postgres:postgres@localhost/deskdispatch").unwrap();
        let s3_credentials = aws_sdk_s3::config::Credentials::new(
            "key",
            "secret",
            None,
            None,
            "static",
        );
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .credentials_provider(s3_credentials)
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

        AppState {
            db: pool,
            s3_client,
            config,
            rate_limiter: LoginRateLimiter::default(),
        }
    }

    #[tokio::test]
    async fn test_security_headers_middleware() {
        let state = create_test_state(Some("http://rustfs:9000".to_string()));
        let app = Router::new()
            .route("/test", get(|| async { "OK" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                security_headers_middleware,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let headers = response.headers();

        let csp = headers.get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap();
        assert!(csp.contains("script-src 'none'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("form-action 'self' http://rustfs:9000"));
        assert!(csp.contains("img-src 'self' data: http://rustfs:9000"));

        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(
            headers.get(header::REFERRER_POLICY).unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert_eq!(
            headers.get(header::STRICT_TRANSPORT_SECURITY).unwrap(),
            "max-age=31536000; includeSubDomains"
        );
    }
}
