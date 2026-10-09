#![forbid(unsafe_code)]

pub mod auth;
pub mod config;
pub mod de;
pub mod domain;
pub mod error;
pub mod magnifier;
pub mod picker;
pub mod retention;
pub mod routes;
pub mod storage;
pub mod validation;

use config::Config;
use secrecy::ExposeSecret;
use sqlx::PgPool;
use axum::{
    middleware,
    routing::get,
    Router,
};
use tower_http::compression::CompressionLayer;

use auth::{csrf_middleware, security_headers_middleware};
use routes::{
    dev_magnifier_verify_handler, livez_handler, not_found_handler, readyz_handler,
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub s3_client: aws_sdk_s3::Client,
    pub config: Config,
    pub rate_limiter: auth::LoginRateLimiter,
    pub task_queue_notifier: tokio::sync::broadcast::Sender<()>,
}

impl AppState {
    pub fn storage_service(&self) -> storage::StorageService {
        storage::StorageService::new(self.s3_client.clone(), &self.config.s3_bucket)
            .with_credentials(
                &self.config.s3_access_key,
                self.config.s3_secret_key.expose_secret(),
                &self.config.s3_region,
                self.config.s3_endpoint.as_deref(),
            )
            .with_public_endpoint(self.config.s3_public_endpoint.as_deref())
    }
}

pub fn build_router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/livez", get(livez_handler))
        .route("/readyz", get(readyz_handler))
        .route("/healthz", get(readyz_handler))
        .merge(routes::static_assets::router())
        .merge(routes::auth::router())
        .merge(routes::account::router())
        .merge(routes::home::router())
        .merge(routes::automations::router())
        .merge(routes::bitmaps::router())
        .merge(routes::media::router())
        .merge(routes::schedules::router())
        .merge(routes::api_workers::router())
        .merge(routes::recordings::router())
        .merge(routes::runs::router())
        .merge(routes::workers::router())
        .merge(routes::users::router());

    if !state.config.is_production() {
        app = app.route("/dev/magnifier-verify", get(dev_magnifier_verify_handler));
    }

    app.fallback(not_found_handler)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers_middleware,
        ))
        .with_state(state)
}
