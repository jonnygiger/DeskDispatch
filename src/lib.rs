pub mod auth;
pub mod config;
pub mod magnifier;
pub mod routes;
pub mod storage;

use config::Config;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub s3_client: aws_sdk_s3::Client,
    pub config: Config,
    pub rate_limiter: auth::LoginRateLimiter,
}
