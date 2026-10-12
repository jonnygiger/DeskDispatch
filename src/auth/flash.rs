use axum::http::HeaderMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FlashLevel {
    Success,
    Error,
    #[default]
    Info,
}

impl FlashLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            FlashLevel::Success => "success",
            FlashLevel::Error => "error",
            FlashLevel::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashMessage {
    pub level: FlashLevel,
    pub message: String,
}

impl FlashMessage {
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            level: FlashLevel::Success,
            message: msg.into(),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            level: FlashLevel::Error,
            message: msg.into(),
        }
    }

    pub fn info(msg: impl Into<String>) -> Self {
        Self {
            level: FlashLevel::Info,
            message: msg.into(),
        }
    }
}

pub fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(encoded, "%{:02X}", b);
            }
        }
    }
    encoded
}

pub fn url_decode(input: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(input.len());
    let mut it = input.bytes();
    while let Some(b) = it.next() {
        match b {
            b'%' => {
                let h1 = it.next()?;
                let h2 = it.next()?;
                let hex_val = |c: u8| -> Option<u8> {
                    match c {
                        b'0'..=b'9' => Some(c - b'0'),
                        b'a'..=b'f' => Some(c - b'a' + 10),
                        b'A'..=b'F' => Some(c - b'A' + 10),
                        _ => None,
                    }
                };
                let v1 = hex_val(h1)?;
                let v2 = hex_val(h2)?;
                bytes.push((v1 << 4) | v2);
            }
            b'+' => bytes.push(b' '),
            b => bytes.push(b),
        }
    }
    String::from_utf8(bytes).ok()
}

pub fn extract_flash_message(headers: &HeaderMap) -> Option<FlashMessage> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    let mut msg: Option<String> = None;
    let mut level: Option<FlashLevel> = None;

    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(val) = cookie.strip_prefix("flash_msg=") {
            if let Some(decoded) = url_decode(val) {
                msg = Some(decoded);
            }
        } else if let Some(val) = cookie.strip_prefix("flash_level=") {
            level = match val {
                "success" => Some(FlashLevel::Success),
                "error" => Some(FlashLevel::Error),
                "info" => Some(FlashLevel::Info),
                _ => None,
            };
        }
    }

    msg.map(|message| FlashMessage {
        level: level.unwrap_or_default(),
        message,
    })
}

pub fn build_flash_cookie(flash: &FlashMessage) -> (String, String) {
    let msg_cookie = format!(
        "flash_msg={}; Path=/; HttpOnly; SameSite=Lax",
        url_encode(&flash.message)
    );
    let level_cookie = format!(
        "flash_level={}; Path=/; HttpOnly; SameSite=Lax",
        flash.level.as_str()
    );
    (msg_cookie, level_cookie)
}

pub fn clear_flash_cookie() -> (String, String) {
    let msg_cookie = "flash_msg=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string();
    let level_cookie = "flash_level=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string();
    (msg_cookie, level_cookie)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_flash_cookie_roundtrip() {
        let flash = FlashMessage::success("Automation saved successfully!");
        let (c1, c2) = build_flash_cookie(&flash);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("{}; {}", c1, c2).parse().unwrap(),
        );

        let extracted = extract_flash_message(&headers).unwrap();
        assert_eq!(extracted.message, "Automation saved successfully!");
        assert_eq!(extracted.level, FlashLevel::Success);
    }
}
