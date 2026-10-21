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
}
