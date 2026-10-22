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

    pub fn mouse_scroll(
        &self,
        direction: &str,
        scroll_amount: i32,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        info!(
            direction = direction,
            scroll_amount = scroll_amount,
            x = x,
            y = y,
            "Executing mouse scroll"
        );

        let btn_num = match direction.to_lowercase().as_str() {
            "up" => "4",
            "down" => "5",
            "left" => "6",
            "right" => "7",
            _ => "5",
        };

        let repeat = scroll_amount.max(1).to_string();

        let mut cmd = Command::new("xdotool");
        cmd.env("DISPLAY", &self.display);

        if let (Some(px), Some(py)) = (x, y) {
            cmd.arg("mousemove").arg(px.to_string()).arg(py.to_string());
        }

        cmd.arg("click").arg("--repeat").arg(repeat).arg(btn_num);

        let status = cmd.status();

        match status {
            Ok(s) if s.success() => Ok(()),
            _ => {
                debug!("xdotool scroll execution omitted or unavailable");
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

    pub fn clipboard_set_text(&self, text: &str) -> Result<(), String> {
        info!(text_len = text.len(), "Setting clipboard text");

        let child = Command::new("xclip")
            .env("DISPLAY", &self.display)
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .spawn();

        if let Ok(mut c) = child {
            if let Some(mut stdin) = c.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = c.wait();
            return Ok(());
        }

        let child_xsel = Command::new("xsel")
            .env("DISPLAY", &self.display)
            .arg("--clipboard")
            .arg("--input")
            .stdin(std::process::Stdio::piped())
            .spawn();

        if let Ok(mut c) = child_xsel {
            if let Some(mut stdin) = c.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = c.wait();
            return Ok(());
        }

        debug!("xclip and xsel unavailable for clipboard set");
        Ok(())
    }

    pub fn clipboard_get_text(&self) -> String {
        let output = Command::new("xclip")
            .env("DISPLAY", &self.display)
            .arg("-selection")
            .arg("clipboard")
            .arg("-out")
            .output();

        if let Ok(out) = output
            && out.status.success()
        {
            return String::from_utf8_lossy(&out.stdout).to_string();
        }

        let output_xsel = Command::new("xsel")
            .env("DISPLAY", &self.display)
            .arg("--clipboard")
            .arg("--output")
            .output();

        if let Ok(out) = output_xsel
            && out.status.success()
        {
            return String::from_utf8_lossy(&out.stdout).to_string();
        }

        String::new()
    }

    pub fn clipboard_clear(&self) -> Result<(), String> {
        self.clipboard_set_text("")
    }

    pub fn clipboard_wait_for_change(&self, timeout_ms: u64) -> Result<String, String> {
        let initial = self.clipboard_get_text();
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let current = self.clipboard_get_text();
            if !current.is_empty() && current != initial {
                return Ok(current);
            }

            if start.elapsed() >= timeout {
                if !current.is_empty() {
                    return Ok(current);
                }
                return Err(format!("Clipboard change timed out after {}ms", timeout_ms));
            }

            std::thread::sleep(std::time::Duration::from_millis(150));
    pub fn check_window_state(
        &self,
        match_pattern: &str,
        match_type: &str,
        check_type: &str,
        timeout_ms: u64,
    ) -> bool {
        info!(
            match_pattern = match_pattern,
            match_type = match_type,
            check_type = check_type,
            timeout_ms = timeout_ms,
            "Checking window state condition"
        );

        let search_arg = match match_type.to_lowercase().as_str() {
            "exact" => format!("^{}$", match_pattern),
            "starts_with" => format!("^{}", match_pattern),
            _ => match_pattern.to_string(),
        };

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let check_active = check_type.to_lowercase() == "active";

        loop {
            if check_active {
                let active_output = Command::new("xdotool")
                    .env("DISPLAY", &self.display)
                    .arg("getactivewindow")
                    .arg("getwindowname")
                    .output();

                if let Ok(output) = active_output
                    && output.status.success()
                {
                    let active_title = String::from_utf8_lossy(&output.stdout);
                    let is_match = match match_type.to_lowercase().as_str() {
                        "exact" => active_title.trim() == match_pattern,
                        "starts_with" => active_title.trim().starts_with(match_pattern),
                        _ => active_title.contains(match_pattern),
                    };

                    if is_match {
                        return true;
                    }
                }
            } else {
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
                    if stdout.lines().any(|l| !l.trim().is_empty()) {
                        return true;
                    }
                }
            }

            if start.elapsed() >= timeout {
                return false;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
