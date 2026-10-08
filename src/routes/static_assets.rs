use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
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
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut response_headers = HeaderMap::new();
            if let Ok(content_type) = mime.as_ref().parse() {
                response_headers.insert(header::CONTENT_TYPE, content_type);
            }

            response_headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("public, max-age=31536000, immutable"),
            );

            let hash = content.metadata.sha256_hash();
            let etag = format!("\"{}\"", hex::encode(hash));

            if let Ok(etag_val) = header::HeaderValue::from_str(&etag) {
                response_headers.insert(header::ETAG, etag_val);
            }

            if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
                if let Ok(req_etag) = if_none_match.to_str() {
                    if req_etag.trim() == etag || req_etag.contains(&etag) {
                        return (StatusCode::NOT_MODIFIED, response_headers).into_response();
                    }
                }
            }

            (StatusCode::OK, response_headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}
