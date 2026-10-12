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

#[tracing::instrument(skip(headers))]
pub async fn static_asset_handler(
    headers: HeaderMap,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    match Assets::get(path) {
        Some(content) => {
            let mime = match path.rsplit('.').next().unwrap_or("") {
                "css" => "text/css; charset=utf-8",
                "js" => "application/javascript; charset=utf-8",
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "svg" => "image/svg+xml",
                "ico" => "image/x-icon",
                "woff2" => "font/woff2",
                _ => "application/octet-stream",
            };
            let mut response_headers = HeaderMap::new();
            if let Ok(content_type) = mime.parse() {
                response_headers.insert(header::CONTENT_TYPE, content_type);
            }

            response_headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("public, max-age=31536000, immutable"),
            );

            let hash = content.metadata.sha256_hash();
            let mut hex_hash = String::with_capacity(64);
            for b in hash {
                use std::fmt::Write;
                let _ = write!(hex_hash, "{:02x}", b);
            }
            let etag = format!("\"{}\"", hex_hash);

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
