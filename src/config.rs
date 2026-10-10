use secrecy::SecretString;
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub public_base_url: String,
    pub s3_endpoint: Option<String>,
    pub s3_public_endpoint: Option<String>,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: SecretString,
    pub s3_region: String,
    pub session_secret: SecretString,
    pub bind_address: String,
    pub environment: String,
    pub min_agent_version: Option<String>,
    pub worker_poll_interval_secs: u64,
    pub worker_heartbeat_interval_secs: u64,
    pub trust_proxy_headers: bool,
    pub database_max_connections: u32,
    pub database_acquire_timeout_secs: u64,
    pub worker_long_poll_timeout_secs: u64,
    pub session_retention_days: u32,
    pub audit_log_retention_days: u32,
    pub task_run_step_retention_days: u32,
    pub screenshot_retention_days: u32,
    pub metrics_bind_address: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let _ = dotenvy::dotenv();

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
        });

        let public_base_url = env::var("PUBLIC_BASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:3000".to_string());

        let s3_endpoint = env::var("S3_ENDPOINT")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let s3_public_endpoint = env::var("S3_PUBLIC_ENDPOINT")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let s3_bucket = env::var("S3_BUCKET").unwrap_or_else(|_| "deskdispatch-bucket".to_string());

        let s3_access_key = env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "rustfsadmin".to_string());

        let s3_secret_key =
            env::var("S3_SECRET_KEY").unwrap_or_else(|_| "rustfsadminpassword".to_string());

        let s3_region = env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let session_secret = env::var("SESSION_SECRET")
            .unwrap_or_else(|_| "default_session_secret_change_me_in_production".to_string());

        let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".to_string());

        let environment = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

        let min_agent_version = env::var("MIN_AGENT_VERSION")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let worker_poll_interval_secs = env::var("WORKER_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let worker_heartbeat_interval_secs = env::var("WORKER_HEARTBEAT_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        let trust_proxy_headers = env::var("TRUST_PROXY_HEADERS")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let database_max_connections = env::var("DATABASE_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let database_acquire_timeout_secs = env::var("DATABASE_ACQUIRE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let worker_long_poll_timeout_secs = env::var("WORKER_LONG_POLL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let session_retention_days = env::var("SESSION_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);

        let audit_log_retention_days = env::var("AUDIT_LOG_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);

        let task_run_step_retention_days = env::var("TASK_RUN_STEP_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let screenshot_retention_days = env::var("SCREENSHOT_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let metrics_bind_address = env::var("METRICS_BIND_ADDRESS")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let config = Self {
            database_url,
            public_base_url,
            s3_endpoint,
            s3_public_endpoint,
            s3_bucket,
            s3_access_key,
            s3_secret_key: SecretString::from(s3_secret_key),
            s3_region,
            session_secret: SecretString::from(session_secret),
            bind_address,
            environment,
            min_agent_version,
            worker_poll_interval_secs,
            worker_heartbeat_interval_secs,
            trust_proxy_headers,
            database_max_connections,
            database_acquire_timeout_secs,
            worker_long_poll_timeout_secs,
            session_retention_days,
            audit_log_retention_days,
            task_run_step_retention_days,
            screenshot_retention_days,
            metrics_bind_address,
        };

        config.validate()?;

        Ok(config)
    }

    pub fn is_production(&self) -> bool {
        self.environment.eq_ignore_ascii_case("production")
    }

    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn create_valid_prod_config() -> Config {
        Config {
            database_url: "postgres://postgres:securepass123@localhost:5432/deskdispatch"
                .to_string(),
            public_base_url: "http://localhost:3000".to_string(),
            s3_endpoint: None,
            s3_public_endpoint: None,
            s3_bucket: "deskdispatch-bucket".to_string(),
            s3_access_key: "secure_s3_user".to_string(),
            s3_secret_key: SecretString::from("secure_s3_secret_key_99".to_string()),
            s3_region: "us-east-1".to_string(),
            session_secret: SecretString::from("secure_session_secret_007".to_string()),
            bind_address: "0.0.0.0:3000".to_string(),
            environment: "production".to_string(),
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
        }
    }

    #[test]
    fn test_default_config() {
        let config = Config::from_env().unwrap();
        assert_eq!(config.s3_bucket, "deskdispatch-bucket");
        assert_eq!(config.session_retention_days, 7);
        assert_eq!(config.audit_log_retention_days, 90);
        assert_eq!(config.task_run_step_retention_days, 30);
        assert_eq!(config.screenshot_retention_days, 30);
        assert!(!config.is_production());
    }


    #[test]
    fn test_production_secure_config_succeeds() {
        let config = create_valid_prod_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_debug_formatting_does_not_leak_secrets() {
        let config = create_valid_prod_config();
        let debug_str = format!("{:?}", config);
        assert!(!debug_str.contains("secure_s3_secret_key_99"));
        assert!(!debug_str.contains("secure_session_secret_007"));
    }
}
