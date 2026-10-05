use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub session_secret: String,
    pub bind_address: String,
    pub environment: String,
    pub min_agent_version: Option<String>,
    pub worker_poll_interval_secs: u64,
    pub worker_heartbeat_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let _ = dotenvy::dotenv();

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());

        let s3_endpoint = env::var("S3_ENDPOINT")
            .ok()
            .or_else(|| Some("http://localhost:9000".to_string()));

        let s3_bucket = env::var("S3_BUCKET")
            .unwrap_or_else(|_| "deskdispatch-bucket".to_string());

        let s3_access_key = env::var("S3_ACCESS_KEY")
            .unwrap_or_else(|_| "rustfsadmin".to_string());

        let s3_secret_key = env::var("S3_SECRET_KEY")
            .unwrap_or_else(|_| "rustfsadminpassword".to_string());

        let s3_region = env::var("S3_REGION")
            .unwrap_or_else(|_| "us-east-1".to_string());

        let session_secret = env::var("SESSION_SECRET")
            .unwrap_or_else(|_| "default_session_secret_change_me_in_production".to_string());

        let bind_address = env::var("BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0:3000".to_string());

        let environment = env::var("APP_ENV")
            .unwrap_or_else(|_| "development".to_string());

        let min_agent_version = env::var("MIN_AGENT_VERSION").ok().filter(|v| !v.trim().is_empty());

        let worker_poll_interval_secs = env::var("WORKER_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let worker_heartbeat_interval_secs = env::var("WORKER_HEARTBEAT_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        Ok(Self {
            database_url,
            s3_endpoint,
            s3_bucket,
            s3_access_key,
            s3_secret_key,
            s3_region,
            session_secret,
            bind_address,
            environment,
            min_agent_version,
            worker_poll_interval_secs,
            worker_heartbeat_interval_secs,
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment.eq_ignore_ascii_case("production")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::from_env().unwrap();
        assert_eq!(config.s3_bucket, "deskdispatch-bucket");
        assert!(!config.is_production());
    }
}
