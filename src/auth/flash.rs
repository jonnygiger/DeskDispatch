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

const HEX_CHARS_UPPER: &[u8; 16] = b"0123456789ABCDEF";

pub fn url_encode(input: &str) -> String {
    let mut encoded = Vec::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b);
            }
            _ => {
                encoded.push(b'%');
                encoded.push(HEX_CHARS_UPPER[(b >> 4) as usize]);
                encoded.push(HEX_CHARS_UPPER[(b & 0x0f) as usize]);
            }
        }
    }
    String::from_utf8(encoded).unwrap_or_default()
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

    #[test]
    fn test_flash_cookie_roundtrip_with_equals_sign() {
        let flash = FlashMessage::error("Failed to process request: status_code=500&error=timeout");
        let (c1, c2) = build_flash_cookie(&flash);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("{}; {}", c1, c2).parse().unwrap(),
        );

        let extracted = extract_flash_message(&headers).unwrap();
        assert_eq!(
            extracted.message,
            "Failed to process request: status_code=500&error=timeout"
        );
        assert_eq!(extracted.level, FlashLevel::Error);
    }

    #[test]
    fn test_url_decode_edge_cases() {
        // Truncated percent escapes
        assert_eq!(url_decode("hello%"), None);
        assert_eq!(url_decode("hello%2"), None);

        // Invalid hex characters
        assert_eq!(url_decode("hello%2G"), None);
        assert_eq!(url_decode("hello%G2"), None);

        // Invalid UTF-8 byte sequences
        assert_eq!(url_decode("%FF%FE"), None);

        // Plus-sign to space conversion and lowercase hex digits
        assert_eq!(
            url_decode("hello+world%2fslash"),
            Some("hello world/slash".to_string())
        );
    }

    #[test]
    fn test_extract_flash_message_edge_cases() {
        // Missing cookie header
        let empty_headers = HeaderMap::new();
        assert_eq!(extract_flash_message(&empty_headers), None);

        // Flash message without explicit level defaults to FlashLevel::Info
        let mut info_headers = HeaderMap::new();
        info_headers.insert(
            axum::http::header::COOKIE,
            "flash_msg=System%20notification".parse().unwrap(),
        );
        let extracted_info = extract_flash_message(&info_headers).unwrap();
        assert_eq!(extracted_info.message, "System notification");
        assert_eq!(extracted_info.level, FlashLevel::Info);

        // Unrecognized flash level cookie value falls back to FlashLevel::Info
        let mut unknown_level_headers = HeaderMap::new();
        unknown_level_headers.insert(
            axum::http::header::COOKIE,
            "flash_msg=Test; flash_level=unknown".parse().unwrap(),
        );
        let extracted_unknown = extract_flash_message(&unknown_level_headers).unwrap();
        assert_eq!(extracted_unknown.message, "Test");
        assert_eq!(extracted_unknown.level, FlashLevel::Info);

        // FlashMessage::info constructor check
        let flash_info = FlashMessage::info("Informational notice");
        assert_eq!(flash_info.level, FlashLevel::Info);
        assert_eq!(flash_info.message, "Informational notice");

        // clear_flash_cookie formatting
        let (c1, c2) = clear_flash_cookie();
        assert!(c1.contains("flash_msg=;"));
        assert!(c1.contains("Max-Age=0"));
        assert!(c2.contains("flash_level=;"));
        assert!(c2.contains("Max-Age=0"));
    }
}
