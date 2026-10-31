use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::wayland::WaylandInput;

/// Which input backend the worker uses.
enum Backend {
    /// XTEST through xdotool. Works on X11 sessions and for Xwayland clients,
    /// but native Wayland clients ignore these events.
    Xdotool,
    /// XDG Desktop Portal RemoteDesktop. Required for native Wayland clients.
    Portal(Arc<PortalState>),
}

struct PortalState {
    display: String,
    screen_width: u32,
    screen_height: u32,
    slot: Mutex<PortalSlot>,
}

struct PortalSlot {
    input: Option<WaylandInput>,
    last_error: Option<(Instant, String)>,
}

pub struct InputController {
    pub display: String,
    backend: Backend,
}

/// How long to wait before retrying a failed portal initialization. This avoids
/// opening a new permission dialog for every single input action.
const PORTAL_RETRY_COOLDOWN: Duration = Duration::from_secs(30);

fn wayland_session_detected() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

async fn portal_input(
    state: &PortalState,
) -> Result<tokio::sync::MutexGuard<'_, PortalSlot>, String> {
    let mut slot = state.slot.lock().await;

    if slot.input.is_some() {
        return Ok(slot);
    }

    if let Some((attempted_at, error)) = &slot.last_error
        && attempted_at.elapsed() < PORTAL_RETRY_COOLDOWN
    {
        return Err(error.clone());
    }

    info!("Initializing Wayland portal input backend");
    match WaylandInput::connect(&state.display, state.screen_width, state.screen_height).await {
        Ok(input) => {
            info!("Wayland portal input backend ready");
            slot.input = Some(input);
            slot.last_error = None;
            Ok(slot)
        }
        Err(error) => {
            warn!("Wayland portal input initialization failed: {}", error);
            slot.last_error = Some((Instant::now(), error.clone()));
            Err(error)
        }
    }
}

fn xdotool_failure(action: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "no error output"
    };
    format!("xdotool {action} failed: {detail}")
}

impl InputController {
    pub fn new(display: &str, screen_width: u32, screen_height: u32) -> Self {
        let requested = std::env::var("DESKDISPATCH_INPUT_BACKEND")
            .unwrap_or_default()
            .to_lowercase();

        let use_portal = match requested.as_str() {
            "xdotool" | "x11" => false,
            "portal" | "wayland" => true,
            _ => wayland_session_detected(),
        };

        let backend = if use_portal {
            let state = Arc::new(PortalState {
                display: display.to_string(),
                screen_width,
                screen_height,
                slot: Mutex::new(PortalSlot {
                    input: None,
                    last_error: None,
                }),
            });

            // Warm up the portal session in the background so the permission
            // dialog (if any) appears at startup instead of during the first
            // automation step.
            let warmup = Arc::clone(&state);
            tokio::spawn(async move {
                match portal_input(&warmup).await {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(
                            "Wayland portal input is not available yet: {}. \
                             It will be retried on the next input action.",
                            error
                        );
                    }
                }
            });

            Backend::Portal(state)
        } else {
            Backend::Xdotool
        };

        Self {
            display: display.to_string(),
            backend,
        }
    }

    pub async fn mouse_click(
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

        if let Backend::Portal(state) = &self.backend {
            let mut slot = portal_input(state).await?;
            let input = slot
                .input
                .as_ref()
                .ok_or_else(|| "Wayland portal input is not initialized".to_string())?;
            return input.mouse_click(x, y, button, click_type).await;
        }

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

        let output = Command::new("xdotool")
            .env("DISPLAY", &self.display)
            .arg("mousemove")
            .arg(x.to_string())
            .arg(y.to_string())
            .arg("click")
            .arg("--repeat")
            .arg(repeat)
            .arg(btn_num)
            .output()
            .map_err(|e| format!("could not run xdotool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(xdotool_failure("click", &output))
        }
    }

    pub async fn mouse_scroll(
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

        if let Backend::Portal(state) = &self.backend {
            let mut slot = portal_input(state).await?;
            let input = slot
                .input
                .as_ref()
                .ok_or_else(|| "Wayland portal input is not initialized".to_string())?;
            return input
                .mouse_scroll(direction, scroll_amount, x, y)
                .await;
        }

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

        let output = cmd
            .output()
            .map_err(|e| format!("could not run xdotool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(xdotool_failure("scroll", &output))
        }
    }

    pub async fn type_text(
        &self,
        text: &str,
        delay_between_keys_ms: u64,
    ) -> Result<(), String> {
        info!(
            text_len = text.len(),
            delay_between_keys_ms = delay_between_keys_ms,
            "Executing type text"
        );

        if let Backend::Portal(state) = &self.backend {
            let mut slot = portal_input(state).await?;
            let input = slot
                .input
                .as_ref()
                .ok_or_else(|| "Wayland portal input is not initialized".to_string())?;
            return input.type_text(text, delay_between_keys_ms).await;
        }

        let mut cmd = Command::new("xdotool");
        cmd.env("DISPLAY", &self.display).arg("type");

        if delay_between_keys_ms > 0 {
            cmd.arg("--delay").arg(delay_between_keys_ms.to_string());
        }

        cmd.arg("--").arg(text);

        let output = cmd
            .output()
            .map_err(|e| format!("could not run xdotool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(xdotool_failure("type", &output))
        }
    }

    pub async fn key_press(&self, key_combo: &str) -> Result<(), String> {
        info!(key_combo = key_combo, "Executing key press combo");

        if let Backend::Portal(state) = &self.backend {
            let mut slot = portal_input(state).await?;
            let input = slot
                .input
                .as_ref()
                .ok_or_else(|| "Wayland portal input is not initialized".to_string())?;
            return input.key_press(key_combo).await;
        }

        let output = Command::new("xdotool")
            .env("DISPLAY", &self.display)
            .arg("key")
            .arg(key_combo)
            .output()
            .map_err(|e| format!("could not run xdotool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(xdotool_failure("key", &output))
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
        }
    }

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
