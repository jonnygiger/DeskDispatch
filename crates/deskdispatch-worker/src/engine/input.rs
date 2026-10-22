use std::process::Command;
use tracing::{debug, info};

pub struct InputController {
    pub display: String,
}

impl InputController {
    pub fn new(display: &str) -> Self {
        Self {
            display: display.to_string(),
        }
    }

    pub fn mouse_click(
        &self,
        x: i32,
        y: i32,
        button: &str,
        click_type: &str,
    ) -> Result<(), String> {
        info!(
            x = x,
            y = y,
            button = button,
            click_type = click_type,
            "Executing mouse click"
        );

        let btn_num = match button.to_lowercase().as_str() {
            "right" => "3",
            "middle" => "2",
            _ => "1",
        };

        let repeat = if click_type.to_lowercase().contains("double") {
            "2"
        } else {
            "1"
        };

        // Try xdotool on Linux Debian
        let status = Command::new("xdotool")
            .env("DISPLAY", &self.display)
            .arg("mousemove")
            .arg(x.to_string())
            .arg(y.to_string())
            .arg("click")
            .arg("--repeat")
            .arg(repeat)
            .arg(btn_num)
            .status();

        match status {
            Ok(s) if s.success() => Ok(()),
            _ => {
                debug!("xdotool click execution omitted or unavailable");
                Ok(())
            }
        }
    }

    pub fn type_text(&self, text: &str, delay_between_keys_ms: u64) -> Result<(), String> {
        info!(
            text_len = text.len(),
            delay_between_keys_ms = delay_between_keys_ms,
            "Executing type text"
        );

        let mut cmd = Command::new("xdotool");
        cmd.env("DISPLAY", &self.display).arg("type");

        if delay_between_keys_ms > 0 {
            cmd.arg("--delay").arg(delay_between_keys_ms.to_string());
        }

        cmd.arg("--").arg(text);

        let status = cmd.status();

        match status {
            Ok(s) if s.success() => Ok(()),
            _ => {
                debug!("xdotool type text execution omitted or unavailable");
                Ok(())
            }
        }
    }

    pub fn key_press(&self, key_combo: &str) -> Result<(), String> {
        info!(key_combo = key_combo, "Executing key press combo");

        let status = Command::new("xdotool")
            .env("DISPLAY", &self.display)
            .arg("key")
            .arg(key_combo)
            .status();

        match status {
            Ok(s) if s.success() => Ok(()),
            _ => {
                debug!("xdotool keypress execution omitted or unavailable");
                Ok(())
            }
        }
    }

    pub fn window_control(
        &self,
        action: &str,
        match_pattern: &str,
        match_type: &str,
        timeout_ms: u64,
    ) -> Result<(), String> {
        info!(
            action = action,
            match_pattern = match_pattern,
            match_type = match_type,
            timeout_ms = timeout_ms,
            "Executing window control"
        );

        let search_arg = match match_type.to_lowercase().as_str() {
            "exact" => format!("^{}$", match_pattern),
            "starts_with" => format!("^{}", match_pattern),
            _ => match_pattern.to_string(),
        };

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let search_output = Command::new("xdotool")
                .env("DISPLAY", &self.display)
                .arg("search")
                .arg("--onlyvisible")
                .arg("--name")
                .arg(&search_arg)
                .output();

            if let Ok(output) = search_output
                && output.status.success()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let window_ids: Vec<&str> = stdout
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();

                if let Some(wid) = window_ids.first() {
                    let act_norm = action.to_lowercase();
                    if act_norm == "wait" {
                        return Ok(());
                    }

                    let status = match act_norm.as_str() {
                        "activate" => Command::new("xdotool")
                            .env("DISPLAY", &self.display)
                            .arg("windowactivate")
                            .arg("--sync")
                            .arg(wid)
                            .status(),
                        "close" => Command::new("xdotool")
                            .env("DISPLAY", &self.display)
                            .arg("windowclose")
                            .arg(wid)
                            .status(),
                        "minimize" => Command::new("xdotool")
                            .env("DISPLAY", &self.display)
                            .arg("windowminimize")
                            .arg(wid)
                            .status(),
                        "maximize" => Command::new("xdotool")
                            .env("DISPLAY", &self.display)
                            .arg("windowsize")
                            .arg(wid)
                            .arg("100%")
                            .arg("100%")
                            .status(),
                        _ => Command::new("xdotool")
                            .env("DISPLAY", &self.display)
                            .arg("windowactivate")
                            .arg(wid)
                            .status(),
                    };

                    match status {
                        Ok(s) if s.success() => return Ok(()),
                        _ => {
                            debug!("xdotool window action execution omitted or unavailable");
                            return Ok(());
                        }
                    }
                }
            }

            if start.elapsed() >= timeout {
                if action.to_lowercase() == "wait" {
                    return Err(format!(
                        "Window matching '{}' not found within {}ms",
                        match_pattern, timeout_ms
                    ));
                }
                debug!("Window search omitted or not found on DISPLAY");
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}
