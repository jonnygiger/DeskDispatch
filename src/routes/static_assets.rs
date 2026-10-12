use axum::{
    Router,
    extract::Path,
    http::{HeaderMap, StatusCode, header},
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

fn get_mime_type(path: &str) -> &'static str {
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

#[tracing::instrument(skip(headers))]
pub async fn static_asset_handler(
    headers: HeaderMap,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    match Assets::get(path) {
        Some(content) => {
            let mime_str = get_mime_type(path);
            let mut response_headers = HeaderMap::new();
            if let Ok(content_type) = mime_str.parse() {
                response_headers.insert(header::CONTENT_TYPE, content_type);
            }

            response_headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("public, max-age=31536000, immutable"),
            );

            let hash = content.metadata.sha256_hash();
            let etag = format!("\"{}\"", crate::storage::encode_hex(&hash));

            if let Ok(etag_val) = header::HeaderValue::from_str(&etag) {
                response_headers.insert(header::ETAG, etag_val);
            }

            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH)
                && let Ok(req_etag) = if_none_match.to_str()
                && (req_etag.trim() == etag || req_etag.contains(&etag))
            {
                return (StatusCode::NOT_MODIFIED, response_headers).into_response();
            }

            (StatusCode::OK, response_headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}
