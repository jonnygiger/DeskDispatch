use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info};
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE_DESKTOP_INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";

const EVDEV_BTN_LEFT: i32 = 0x110;
const EVDEV_BTN_RIGHT: i32 = 0x111;
const EVDEV_BTN_MIDDLE: i32 = 0x112;

/// Wayland input controller using XDG Desktop Portal RemoteDesktop.
pub struct WaylandInput {
    connection: Connection,
    session_handle: OwnedObjectPath,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl WaylandInput {
    pub async fn connect(
        _display: &str,
        screen_width: u32,
        screen_height: u32,
    ) -> Result<Self, String> {
        info!("Connecting to session bus for Wayland RemoteDesktop portal");
        let connection = Connection::session()
            .await
            .map_err(|e| format!("Failed to connect to D-Bus session bus: {e}"))?;

        let mut create_options: HashMap<&str, Value<'_>> = HashMap::new();
        create_options.insert(
            "session_handle_token",
            Value::from("deskdispatch_worker_session"),
        );
        create_options.insert("handle_token", Value::from("deskdispatch_create_session"));

        let reply = connection
            .call_method(
                Some(PORTAL_DESTINATION),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_INTERFACE),
                "CreateSession",
                &(create_options,),
            )
            .await
            .map_err(|e| format!("CreateSession call failed: {e}"))?;

        let session_handle: OwnedObjectPath = reply
            .body()
            .deserialize()
            .map_err(|e| format!("Failed to deserialize session handle: {e}"))?;

        debug!(session = %session_handle, "Created RemoteDesktop session");

        // Select devices: 1 = Keyboard, 2 = Pointer -> bitmask 3
        let mut select_options: HashMap<&str, Value<'_>> = HashMap::new();
        select_options.insert("types", Value::from(3u32));
        select_options.insert("handle_token", Value::from("deskdispatch_select_devices"));

        let _ = connection
            .call_method(
                Some(PORTAL_DESTINATION),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_INTERFACE),
                "SelectDevices",
                &(&session_handle, select_options),
            )
            .await
            .map_err(|e| format!("SelectDevices call failed: {e}"))?;

        // Start session
        let mut start_options: HashMap<&str, Value<'_>> = HashMap::new();
        start_options.insert("handle_token", Value::from("deskdispatch_start_session"));

        let _ = connection
            .call_method(
                Some(PORTAL_DESTINATION),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_INTERFACE),
                "Start",
                &(&session_handle, "", start_options),
            )
            .await
            .map_err(|e| format!("Start session call failed: {e}"))?;

        Ok(Self {
            connection,
            session_handle,
            screen_width: screen_width.max(1),
            screen_height: screen_height.max(1),
        })
    }

    pub async fn mouse_click(
        &self,
        x: i32,
        y: i32,
        button: &str,
        click_type: &str,
    ) -> Result<(), String> {
        let btn_code = match button.to_lowercase().as_str() {
            "right" => EVDEV_BTN_RIGHT,
            "middle" => EVDEV_BTN_MIDDLE,
            _ => EVDEV_BTN_LEFT,
        };

        let repeat = if click_type.to_lowercase().contains("double") {
            2
        } else {
            1
        };

        // Move cursor to absolute position
        let stream = 0u32;
        let empty_options: HashMap<&str, Value<'_>> = HashMap::new();

        self.connection
            .call_method(
                Some(PORTAL_DESTINATION),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_INTERFACE),
                "NotifyPointerMotionAbsolute",
                &(
                    &self.session_handle,
                    &empty_options,
                    stream,
                    x as f64,
                    y as f64,
                ),
            )
            .await
            .map_err(|e| format!("NotifyPointerMotionAbsolute failed: {e}"))?;

        for i in 0..repeat {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // Press button (state = 1)
            self.connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyPointerButton",
                    &(&self.session_handle, &empty_options, btn_code, 1u32),
                )
                .await
                .map_err(|e| format!("NotifyPointerButton press failed: {e}"))?;

            tokio::time::sleep(Duration::from_millis(30)).await;

            // Release button (state = 0)
            self.connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyPointerButton",
                    &(&self.session_handle, &empty_options, btn_code, 0u32),
                )
                .await
                .map_err(|e| format!("NotifyPointerButton release failed: {e}"))?;
        }

        Ok(())
    }

    pub async fn mouse_scroll(
        &self,
        direction: &str,
        scroll_amount: i32,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        let empty_options: HashMap<&str, Value<'_>> = HashMap::new();

        if let (Some(px), Some(py)) = (x, y) {
            let stream = 0u32;
            let _ = self
                .connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyPointerMotionAbsolute",
                    &(
                        &self.session_handle,
                        &empty_options,
                        stream,
                        px as f64,
                        py as f64,
                    ),
                )
                .await;
        }

        let (axis, steps) = match direction.to_lowercase().as_str() {
            "up" => (0u32, -scroll_amount.max(1)),
            "down" => (0u32, scroll_amount.max(1)),
            "left" => (1u32, -scroll_amount.max(1)),
            "right" => (1u32, scroll_amount.max(1)),
            _ => (0u32, scroll_amount.max(1)),
        };

        self.connection
            .call_method(
                Some(PORTAL_DESTINATION),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_INTERFACE),
                "NotifyPointerAxisDiscrete",
                &(&self.session_handle, &empty_options, axis, steps),
            )
            .await
            .map_err(|e| format!("NotifyPointerAxisDiscrete failed: {e}"))?;

        Ok(())
    }

    pub async fn type_text(&self, text: &str, delay_between_keys_ms: u64) -> Result<(), String> {
        let empty_options: HashMap<&str, Value<'_>> = HashMap::new();

        for ch in text.chars() {
            let keysym = char_to_keysym(ch);
            if keysym != 0 {
                // Press
                self.connection
                    .call_method(
                        Some(PORTAL_DESTINATION),
                        PORTAL_PATH,
                        Some(REMOTE_DESKTOP_INTERFACE),
                        "NotifyKeyboardKeysym",
                        &(&self.session_handle, &empty_options, keysym, 1u32),
                    )
                    .await
                    .map_err(|e| format!("NotifyKeyboardKeysym press failed: {e}"))?;

                tokio::time::sleep(Duration::from_millis(10)).await;

                // Release
                self.connection
                    .call_method(
                        Some(PORTAL_DESTINATION),
                        PORTAL_PATH,
                        Some(REMOTE_DESKTOP_INTERFACE),
                        "NotifyKeyboardKeysym",
                        &(&self.session_handle, &empty_options, keysym, 0u32),
                    )
                    .await
                    .map_err(|e| format!("NotifyKeyboardKeysym release failed: {e}"))?;
            }

            if delay_between_keys_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_between_keys_ms)).await;
            }
        }

        Ok(())
    }

    pub async fn key_press(&self, key_combo: &str) -> Result<(), String> {
        let empty_options: HashMap<&str, Value<'_>> = HashMap::new();
        let (modifiers, main_key) = parse_key_combo(key_combo);

        // Press modifiers
        for &mod_sym in &modifiers {
            self.connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyKeyboardKeysym",
                    &(&self.session_handle, &empty_options, mod_sym, 1u32),
                )
                .await
                .map_err(|e| format!("NotifyKeyboardKeysym modifier press failed: {e}"))?;
        }

        // Press main key
        if let Some(main_sym) = main_key {
            self.connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyKeyboardKeysym",
                    &(&self.session_handle, &empty_options, main_sym, 1u32),
                )
                .await
                .map_err(|e| format!("NotifyKeyboardKeysym main key press failed: {e}"))?;

            tokio::time::sleep(Duration::from_millis(20)).await;

            // Release main key
            self.connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyKeyboardKeysym",
                    &(&self.session_handle, &empty_options, main_sym, 0u32),
                )
                .await
                .map_err(|e| format!("NotifyKeyboardKeysym main key release failed: {e}"))?;
        }

        // Release modifiers in reverse order
        for &mod_sym in modifiers.iter().rev() {
            let _ = self
                .connection
                .call_method(
                    Some(PORTAL_DESTINATION),
                    PORTAL_PATH,
                    Some(REMOTE_DESKTOP_INTERFACE),
                    "NotifyKeyboardKeysym",
                    &(&self.session_handle, &empty_options, mod_sym, 0u32),
                )
                .await;
        }

        Ok(())
    }
}

pub fn char_to_keysym(ch: char) -> i32 {
    let cp = ch as u32;
    // Standard ASCII / Latin-1 mapping directly into X11 keysyms
    if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) {
        return cp as i32;
    }
    match ch {
        '\n' | '\r' => 0xff0d, // Return
        '\t' => 0xff09,        // Tab
        '\x08' => 0xff08,      // BackSpace
        '\x1b' => 0xff1b,      // Escape
        _ => {
            // Unicode keysym offset
            (cp | 0x01000000) as i32
        }
    }
}

pub fn key_name_to_keysym(name: &str) -> Option<i32> {
    match name.to_lowercase().as_str() {
        "ctrl" | "control" | "leftctrl" => Some(0xffe3),
        "rightctrl" => Some(0xffe4),
        "alt" | "leftalt" => Some(0xffe9),
        "rightalt" | "altgr" => Some(0xffea),
        "shift" | "leftshift" => Some(0xffe1),
        "rightshift" => Some(0xffe2),
        "super" | "win" | "meta" | "leftmeta" => Some(0xffeb),
        "rightmeta" => Some(0xffec),
        "enter" | "return" => Some(0xff0d),
        "tab" => Some(0xff09),
        "backspace" => Some(0xff08),
        "escape" | "esc" => Some(0xff1b),
        "space" => Some(0x0020),
        "delete" | "del" => Some(0xffff),
        "insert" => Some(0xff63),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        "pageup" | "prior" => Some(0xff55),
        "pagedown" | "next" => Some(0xff56),
        "up" => Some(0xff52),
        "down" => Some(0xff54),
        "left" => Some(0xff51),
        "right" => Some(0xff53),
        "f1" => Some(0xffbe),
        "f2" => Some(0xffbf),
        "f3" => Some(0xffc0),
        "f4" => Some(0xffc1),
        "f5" => Some(0xffc2),
        "f6" => Some(0xffc3),
        "f7" => Some(0xffc4),
        "f8" => Some(0xffc5),
        "f9" => Some(0xffc6),
        "f10" => Some(0xffc7),
        "f11" => Some(0xffc8),
        "f12" => Some(0xffc9),
        other => {
            if other.len() == 1 {
                other.chars().next().map(char_to_keysym)
            } else {
                None
            }
        }
    }
}

pub fn parse_key_combo(combo: &str) -> (Vec<i32>, Option<i32>) {
    let parts: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut modifiers = Vec::new();
    let mut main_key = None;

    for part in parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers.push(0xffe3),
            "alt" => modifiers.push(0xffe9),
            "shift" => modifiers.push(0xffe1),
            "super" | "win" | "meta" => modifiers.push(0xffeb),
            _ => {
                if let Some(sym) = key_name_to_keysym(part) {
                    main_key = Some(sym);
                }
            }
        }
    }

    (modifiers, main_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_to_keysym() {
        assert_eq!(char_to_keysym('a'), 97);
        assert_eq!(char_to_keysym('Z'), 90);
        assert_eq!(char_to_keysym('\n'), 0xff0d);
        assert_eq!(char_to_keysym('\t'), 0xff09);
        assert_eq!(char_to_keysym(' '), 32);
    }

    #[test]
    fn test_key_combo_parsing() {
        let (mods, main) = parse_key_combo("ctrl+c");
        assert_eq!(mods, vec![0xffe3]);
        assert_eq!(main, Some(99));

        let (mods, main) = parse_key_combo("ctrl+alt+del");
        assert_eq!(mods, vec![0xffe3, 0xffe9]);
        assert_eq!(main, Some(0xffff));

        let (mods, main) = parse_key_combo("F5");
        assert!(mods.is_empty());
        assert_eq!(main, Some(0xffc2));
    }
}
