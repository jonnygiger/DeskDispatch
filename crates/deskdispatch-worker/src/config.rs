use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug, Clone)]
#[command(name = "deskdispatch-worker")]
#[command(about = "DeskDispatch Worker PC Client for x86 Debian Linux", long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to JSON configuration file
    #[arg(short = 'c', long, env = "WORKER_CONFIG_PATH")]
    pub config: Option<PathBuf>,

    /// DeskDispatch Task Server URL
    #[arg(short = 's', long, env = "DESKDISPATCH_SERVER_URL")]
    pub server_url: Option<String>,

    /// WPS Pre-Shared Key for unattended registration
    #[arg(long, env = "DESKDISPATCH_PSK")]
    pub psk: Option<String>,

    /// One-time registration token
    #[arg(short = 't', long, env = "DESKDISPATCH_REGISTRATION_TOKEN")]
    pub token: Option<String>,

    /// Machine API key (if already registered)
    #[arg(short = 'k', long, env = "DESKDISPATCH_API_KEY")]
    pub api_key: Option<String>,

    /// Worker PC hostname
    #[arg(long, env = "WORKER_HOSTNAME")]
    pub hostname: Option<String>,

    /// Worker PC display name
    #[arg(long, env = "WORKER_DISPLAY_NAME")]
    pub display_name: Option<String>,

    /// X11 DISPLAY environment setting
    #[arg(long, env = "DISPLAY", default_value = ":0")]
    pub display: String,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Start the worker task execution agent loop
    Run,

    /// Generate default configuration file
    ConfigGen {
        /// Output path for generated JSON configuration file
        #[arg(short, long, default_value = "/etc/deskdispatch-worker/config.json")]
        output: PathBuf,
    },

    /// Manage systemd service on Debian Linux
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServiceAction {
    /// Install and enable systemd service unit
    Install {
        /// Systemd service unit name
        #[arg(long, default_value = "deskdispatch-worker")]
        name: String,

        /// System user to run the service under
        #[arg(long, default_value = "root")]
        user: String,
    },
    /// Uninstall and disable systemd service unit
    Uninstall {
        /// Systemd service unit name
        #[arg(long, default_value = "deskdispatch-worker")]
        name: String,
    },
    /// Query systemd service status
    Status {
        /// Systemd service unit name
        #[arg(long, default_value = "deskdispatch-worker")]
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub server_url: String,
    pub psk: Option<String>,
    pub registration_token: Option<String>,
    pub api_key: Option<String>,
    pub hostname: String,
    pub display_name: String,
    pub display: String,
    pub log_level: String,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "debian-worker-pc".to_string());

        Self {
            server_url: "http://localhost:3000".to_string(),
            psk: None,
            registration_token: None,
            api_key: None,
            display_name: format!("Worker Node ({})", hostname),
            hostname,
            display: ":0".to_string(),
            log_level: "info".to_string(),
            screen_width: 1920,
            screen_height: 1080,
        }
    }
}

impl WorkerConfig {
    pub fn load(args: &CliArgs) -> Result<Self, String> {
        let mut config = if let Some(ref path) = args.config {
            Self::from_file(path)?
        } else if Path::new("/etc/deskdispatch-worker/config.json").exists() {
            Self::from_file(Path::new("/etc/deskdispatch-worker/config.json"))?
        } else {
            Self::default()
        };

        if let Some(ref url) = args.server_url {
            config.server_url = url.trim_end_matches('/').to_string();
        }
        if let Some(ref psk) = args.psk {
            config.psk = Some(psk.clone());
        }
        if let Some(ref tok) = args.token {
            config.registration_token = Some(tok.clone());
        }
        if let Some(ref key) = args.api_key {
            config.api_key = Some(key.clone());
        }
        if let Some(ref host) = args.hostname {
            config.hostname = host.clone();
        }
        if let Some(ref disp) = args.display_name {
            config.display_name = disp.clone();
        }
        if !args.display.is_empty() {
            config.display = args.display.clone();
        }
        if !args.log_level.is_empty() {
            config.log_level = args.log_level.clone();
        }

        Ok(config)
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {}", path.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config JSON from {}: {}", path.display(), e))
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create config directory {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize worker config: {}", e))?;
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write config file {}: {}", path.display(), e))
    }
}
