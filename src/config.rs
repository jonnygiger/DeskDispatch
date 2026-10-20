use std::collections::HashMap;
use std::env;
use std::fmt;

/// Zero-dependency secret string wrapper that redacts sensitive string content
/// when formatted with `Debug`.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    #[inline]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl From<String> for SecretString {
    #[inline]
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SecretString {
    #[inline]
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString(***)")
    }
}

/// Zero-dependency `.env` file parser returning key-value pairs without modifying process environment state.
pub fn parse_dotenv() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(".env") else {
        return map;
    };
    parse_dotenv_content(&content, &mut map);
    map
}

pub fn parse_dotenv_content(content: &str, map: &mut HashMap<String, String>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            let unquoted = if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            map.insert(key.to_string(), unquoted.to_string());
        }
    }
}

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
    pub unattended_registration_psk: SecretString,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let dotenv_vars = parse_dotenv();

        let get_var = |key: &str| -> Option<String> {
            env::var(key)
                .ok()
                .or_else(|| dotenv_vars.get(key).cloned())
                .filter(|v| !v.trim().is_empty())
        };

        let database_url = get_var("DATABASE_URL").unwrap_or_else(|| {
            "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
        });

        let public_base_url =
            get_var("PUBLIC_BASE_URL").unwrap_or_else(|| "http://localhost:3000".to_string());

        let s3_endpoint = get_var("S3_ENDPOINT");

        let s3_public_endpoint = get_var("S3_PUBLIC_ENDPOINT");

        let s3_bucket = get_var("S3_BUCKET").unwrap_or_else(|| "deskdispatch-bucket".to_string());

        let s3_access_key = get_var("S3_ACCESS_KEY").unwrap_or_else(|| "rustfsadmin".to_string());

        let s3_secret_key =
            get_var("S3_SECRET_KEY").unwrap_or_else(|| "rustfsadminpassword".to_string());

        let s3_region = get_var("S3_REGION").unwrap_or_else(|| "us-east-1".to_string());

        let session_secret = get_var("SESSION_SECRET")
            .unwrap_or_else(|| "default_session_secret_change_me_in_production".to_string());

        let bind_address = get_var("BIND_ADDRESS").unwrap_or_else(|| "0.0.0.0:3000".to_string());

        let environment = get_var("APP_ENV").unwrap_or_else(|| "development".to_string());

        let min_agent_version = get_var("MIN_AGENT_VERSION");

        let worker_poll_interval_secs = get_var("WORKER_POLL_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let worker_heartbeat_interval_secs = get_var("WORKER_HEARTBEAT_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        let trust_proxy_headers = get_var("TRUST_PROXY_HEADERS")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let database_max_connections = get_var("DATABASE_MAX_CONNECTIONS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let database_acquire_timeout_secs = get_var("DATABASE_ACQUIRE_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let worker_long_poll_timeout_secs = get_var("WORKER_LONG_POLL_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let session_retention_days = get_var("SESSION_RETENTION_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);

        let audit_log_retention_days = get_var("AUDIT_LOG_RETENTION_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);

        let task_run_step_retention_days = get_var("TASK_RUN_STEP_RETENTION_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let screenshot_retention_days = get_var("SCREENSHOT_RETENTION_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let metrics_bind_address = get_var("METRICS_BIND_ADDRESS");

        let unattended_registration_psk = get_var("UNATTENDED_REGISTRATION_PSK")
            .or_else(|| get_var("STATIC_PSK"))
            .unwrap_or_else(|| "deskdispatch_wps_psk_secret".to_string());

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
            unattended_registration_psk: SecretString::from(unattended_registration_psk),
        };

        config.validate()?;

        Ok(config)
    }

    pub fn is_production(&self) -> bool {
        self.environment.eq_ignore_ascii_case("production")
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.is_production() {
            if self.session_secret.expose_secret()
                == "default_session_secret_change_me_in_production"
            {
                return Err(
                    "In production mode, SESSION_SECRET must be set to a custom secret."
                        .to_string(),
                );
            }
            if self.s3_access_key == "rustfsadmin"
                || self.s3_secret_key.expose_secret() == "rustfsadminpassword"
            {
                return Err(
                    "In production mode, default S3 credentials cannot be used.".to_string()
                );
            }
        }
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
            unattended_registration_psk: SecretString::from(
                "deskdispatch_wps_psk_secret".to_string(),
            ),
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
    fn test_production_config_rejects_default_credentials() {
        let mut config = create_valid_prod_config();
        config.session_secret =
            SecretString::from("default_session_secret_change_me_in_production".to_string());
        assert!(config.validate().is_err());

        let mut config2 = create_valid_prod_config();
        config2.s3_access_key = "rustfsadmin".to_string();
        assert!(config2.validate().is_err());

        let mut config3 = create_valid_prod_config();
        config3.s3_secret_key = SecretString::from("rustfsadminpassword".to_string());
        assert!(config3.validate().is_err());
    }

    #[test]
    fn test_parse_dotenv_content() {
        let env_content = r#"
# Comment line
TEST_DESKDISPATCH_PORT=8080
TEST_DESKDISPATCH_NAME="DeskDispatch Service"
TEST_DESKDISPATCH_SINGLE='SingleQuoted'
TEST_DESKDISPATCH_QUOTE_CHAR="
TEST_DESKDISPATCH_APOS_CHAR='
"#;
        let mut map = HashMap::new();
        parse_dotenv_content(env_content, &mut map);

        assert_eq!(
            map.get("TEST_DESKDISPATCH_PORT").map(String::as_str),
            Some("8080")
        );
        assert_eq!(
            map.get("TEST_DESKDISPATCH_NAME").map(String::as_str),
            Some("DeskDispatch Service")
        );
        assert_eq!(
            map.get("TEST_DESKDISPATCH_SINGLE").map(String::as_str),
            Some("SingleQuoted")
        );
        assert_eq!(
            map.get("TEST_DESKDISPATCH_QUOTE_CHAR").map(String::as_str),
            Some("\"")
        );
        assert_eq!(
            map.get("TEST_DESKDISPATCH_APOS_CHAR").map(String::as_str),
            Some("'")
        );
    }

    #[test]
    fn test_debug_formatting_does_not_leak_secrets() {
        let config = create_valid_prod_config();
        let debug_str = format!("{:?}", config);
        assert!(!debug_str.contains("secure_s3_secret_key_99"));
        assert!(!debug_str.contains("secure_session_secret_007"));
    }

    #[test]
    fn test_parse_dotenv_content_edge_cases() {
        // Arrange: dotenv content with comments containing leading spaces, key/value surrounding whitespace,
        // multiple equals signs, empty values, empty quoted strings, mismatched quotes, single character strings,
        // and lines without equals signs.
        let env_content = r#"
   # Leading whitespace comment
NO_EQUALS_LINE
  SPACED_KEY   =   spaced_value
EMPTY_VAL=
EMPTY_QUOTED_DOUBLE=""
EMPTY_QUOTED_SINGLE=''
MISMATCHED_QUOTES="unmatched'
MULTIPLE_EQUALS=postgres://user:pass@localhost:5432/db?ssl=true&mode=1
SINGLE_CHAR_QUOTED="x"
"#;
        let mut map = HashMap::new();

        // Act
        parse_dotenv_content(env_content, &mut map);

        // Assert: Verify expected parsed entries and omission of invalid or comment lines
        assert_eq!(map.len(), 7);
        assert_eq!(
            map.get("SPACED_KEY").map(String::as_str),
            Some("spaced_value")
        );
        assert_eq!(map.get("EMPTY_VAL").map(String::as_str), Some(""));
        assert_eq!(map.get("EMPTY_QUOTED_DOUBLE").map(String::as_str), Some(""));
        assert_eq!(map.get("EMPTY_QUOTED_SINGLE").map(String::as_str), Some(""));
        assert_eq!(
            map.get("MISMATCHED_QUOTES").map(String::as_str),
            Some("\"unmatched'")
        );
        assert_eq!(
            map.get("MULTIPLE_EQUALS").map(String::as_str),
            Some("postgres://user:pass@localhost:5432/db?ssl=true&mode=1")
        );
        assert_eq!(map.get("SINGLE_CHAR_QUOTED").map(String::as_str), Some("x"));
        assert!(!map.contains_key("NO_EQUALS_LINE"));
    }
}
