use axum::{
    Router,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use rust_embed::RustEmbed;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/static/{*path}", get(static_asset_handler))
}

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct Assets;

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Zero-allocation helper to format a 256-bit SHA-256 hash array into a quoted ETag ASCII byte buffer
/// `[0x22, hex_0, hex_1, ..., hex_63, 0x22]` and a pre-constructed `HeaderValue`.
#[inline]
fn format_etag(hash: &[u8; 32]) -> ([u8; 66], HeaderValue) {
    let mut buf = [0u8; 66];
    buf[0] = b'"';
    for (i, &b) in hash.iter().enumerate() {
        buf[1 + i * 2] = HEX_CHARS[(b >> 4) as usize];
        buf[2 + i * 2] = HEX_CHARS[(b & 0x0f) as usize];
    }
    buf[65] = b'"';
    // Safety: buf contains only ASCII '"' and lowercase hex digits
    let header_val = HeaderValue::from_bytes(&buf).expect("valid ASCII ETag header value");
    (buf, header_val)
}

/// Returns the static string slice corresponding to the file extension of `path`.
#[inline]
pub fn get_mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or_default();
    match ext.to_ascii_lowercase().as_str() {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Returns the static `HeaderValue` corresponding to the file extension of `path`.
#[inline]
fn get_mime_header_value(path: &str) -> HeaderValue {
    HeaderValue::from_static(get_mime_type(path))
}

#[tracing::instrument(skip(headers))]
pub async fn static_asset_handler(
    headers: HeaderMap,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let clean_path = path.trim_start_matches('/');
    match Assets::get(clean_path) {
        Some(content) => {
            let (etag_buf, etag_val) = format_etag(&content.metadata.sha256_hash());
            let etag_str = std::str::from_utf8(&etag_buf).unwrap_or_default();

            let mut response_headers = HeaderMap::with_capacity(3);
            response_headers.insert(header::CONTENT_TYPE, get_mime_header_value(clean_path));
            response_headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
            response_headers.insert(header::ETAG, etag_val);

            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH)
                && let Ok(req_etag) = if_none_match.to_str()
                && (req_etag.trim() == etag_str || req_etag.contains(etag_str))
            {
                return (StatusCode::NOT_MODIFIED, response_headers).into_response();
            }

            (StatusCode::OK, response_headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};

    #[test]
    fn test_get_mime_type_extension_resolution() {
        assert_eq!(get_mime_type("style.css"), "text/css; charset=utf-8");
        assert_eq!(
            get_mime_type("script.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(get_mime_type("logo.png"), "image/png");
        assert_eq!(get_mime_type("photo.jpg"), "image/jpeg");
        assert_eq!(get_mime_type("photo.jpeg"), "image/jpeg");
        assert_eq!(get_mime_type("anim.gif"), "image/gif");
        assert_eq!(get_mime_type("icon.svg"), "image/svg+xml");
        assert_eq!(get_mime_type("favicon.ico"), "image/x-icon");
        assert_eq!(get_mime_type("banner.webp"), "image/webp");
        assert_eq!(get_mime_type("font.woff2"), "font/woff2");
        assert_eq!(get_mime_type("font.woff"), "font/woff");
        assert_eq!(get_mime_type("font.ttf"), "font/ttf");
        assert_eq!(get_mime_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(get_mime_type("data.json"), "application/json");

        // Case insensitivity
        assert_eq!(get_mime_type("MAIN.CSS"), "text/css; charset=utf-8");
        assert_eq!(
            get_mime_type("App.JS"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(get_mime_type("Hero.PNG"), "image/png");

        // Multi-dot paths
        assert_eq!(get_mime_type("styles.min.css"), "text/css; charset=utf-8");
        assert_eq!(
            get_mime_type("bundle.v2.js"),
            "application/javascript; charset=utf-8"
        );

        // Missing / unknown extensions
        assert_eq!(get_mime_type("README"), "application/octet-stream");
        assert_eq!(get_mime_type("binary.dat"), "application/octet-stream");
        assert_eq!(get_mime_type(".gitignore"), "application/octet-stream");
    }

    #[test]
    fn test_format_etag() {
        let dummy_hash = [0xab_u8; 32];
        let (buf, val) = format_etag(&dummy_hash);
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s.len(), 66);
        assert!(s.starts_with('"'));
        assert!(s.ends_with('"'));
        assert_eq!(val.to_str().unwrap(), s);
    }

    #[tokio::test]
    async fn test_static_asset_handler_existing_asset_and_404() {
        let headers = HeaderMap::new();

        // 1. Existing asset ("style.css")
        let res = static_asset_handler(headers.clone(), Path("style.css".to_string()))
            .await
            .into_response();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        assert!(res.headers().get(header::ETAG).is_some());

        // 2. Existing asset with leading slash ("/style.css")
        let res_slash = static_asset_handler(headers, Path("/style.css".to_string()))
            .await
            .into_response();
        assert_eq!(res_slash.status(), StatusCode::OK);

        // 3. Non-existent asset ("missing.css")
        let res_404 = static_asset_handler(HeaderMap::new(), Path("missing.css".to_string()))
            .await
            .into_response();
        assert_eq!(res_404.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_static_asset_handler_conditional_caching() {
        // Fetch original asset to extract generated ETag
        let orig_res = static_asset_handler(HeaderMap::new(), Path("style.css".to_string()))
            .await
            .into_response();
        let etag_val = orig_res
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Match exact ETag -> 304 NOT_MODIFIED
        let mut match_headers = HeaderMap::new();
        match_headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&etag_val).unwrap(),
        );
        let res_304 = static_asset_handler(match_headers, Path("style.css".to_string()))
            .await
            .into_response();
        assert_eq!(res_304.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            res_304.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );

        // Match ETag in comma-separated or weak list -> 304 NOT_MODIFIED
        let mut list_headers = HeaderMap::new();
        let list_etag = format!("\"other-etag\", {etag_val}");
        list_headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&list_etag).unwrap(),
        );
        let res_list_304 = static_asset_handler(list_headers, Path("style.css".to_string()))
            .await
            .into_response();
        assert_eq!(res_list_304.status(), StatusCode::NOT_MODIFIED);

        // Mismatched ETag -> 200 OK
        let mut mismatch_headers = HeaderMap::new();
        mismatch_headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"outdated-etag-12345\""),
        );
        let res_200 = static_asset_handler(mismatch_headers, Path("style.css".to_string()))
            .await
            .into_response();
        assert_eq!(res_200.status(), StatusCode::OK);
    }
}
