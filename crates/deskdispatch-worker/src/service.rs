use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

pub struct SystemdServiceManager {
    pub name: String,
    pub user: String,
}

impl SystemdServiceManager {
    pub fn new(name: &str, user: &str) -> Self {
        Self {
            name: name.to_string(),
            user: user.to_string(),
        }
    }

    pub fn service_file_path(&self) -> PathBuf {
        PathBuf::from(format!("/etc/systemd/system/{}.service", self.name))
    }

    pub fn generate_unit_file(&self, binary_path: &Path, config_path: Option<&Path>) -> String {
        let exec_start = match config_path {
            Some(p) => format!("{} --config {} run", binary_path.display(), p.display()),
            None => format!("{} run", binary_path.display()),
        };

        format!(
            r#"[Unit]
Description=DeskDispatch Task Worker PC Client Service
After=network-online.target graphical.target
Wants=network-online.target

[Service]
Type=simple
User={}
Environment="DISPLAY=:0"
Environment="RUST_LOG=info"
ExecStart={}
Restart=always
RestartSec=10s
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
"#,
            self.user, exec_start
        )
    }

    pub fn install(&self, binary_path: &Path, config_path: Option<&Path>) -> Result<(), String> {
        let unit_content = self.generate_unit_file(binary_path, config_path);
        let target_path = self.service_file_path();

        if let Some(parent) = target_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        std::fs::write(&target_path, unit_content).map_err(|e| {
            format!(
                "Failed to write systemd unit file to {}: {}",
                target_path.display(),
                e
            )
        })?;

        info!("Created systemd service unit at {}", target_path.display());

        Self::run_systemctl(&["daemon-reload"])?;
        Self::run_systemctl(&["enable", &self.name])?;
        Self::run_systemctl(&["restart", &self.name])?;

        info!("Successfully installed and started service '{}'", self.name);
        Ok(())
    }

    pub fn uninstall(&self) -> Result<(), String> {
        let target_path = self.service_file_path();

        let _ = Self::run_systemctl(&["stop", &self.name]);
        let _ = Self::run_systemctl(&["disable", &self.name]);

        if target_path.exists() {
            std::fs::remove_file(&target_path).map_err(|e| {
                format!(
                    "Failed to remove systemd unit file {}: {}",
                    target_path.display(),
                    e
                )
            })?;
            info!("Removed service unit file {}", target_path.display());
        } else {
            warn!("Service unit file {} does not exist", target_path.display());
        }

        Self::run_systemctl(&["daemon-reload"])?;
        info!("Successfully uninstalled service '{}'", self.name);
        Ok(())
    }

    pub fn status(&self) -> Result<String, String> {
        let output = Command::new("systemctl")
            .arg("status")
            .arg(&self.name)
            .output()
            .map_err(|e| format!("Failed to execute 'systemctl status': {}", e))?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn run_systemctl(args: &[&str]) -> Result<(), String> {
        let status = Command::new("systemctl")
            .args(args)
            .status()
            .map_err(|e| format!("Failed to execute systemctl {:?}: {}", args, e))?;

        if !status.success() {
            return Err(format!(
                "systemctl {:?} exited with code {:?}",
                args,
                status.code()
            ));
        }
        Ok(())
    }
}
