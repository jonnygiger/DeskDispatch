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

impl AppState {
    pub fn storage_service(&self) -> storage::StorageService {
        storage::StorageService::new(self.s3_client.clone(), &self.config.s3_bucket)
            .with_credentials(
                &self.config.s3_access_key,
                &self.config.s3_secret_key,
                &self.config.s3_region,
                self.config.s3_endpoint.as_deref(),
            )
    }
}
