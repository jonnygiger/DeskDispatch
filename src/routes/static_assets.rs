use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct Assets;

#[tracing::instrument]
pub async fn static_asset_handler(Path(path): Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut headers = HeaderMap::new();
            if let Ok(content_type) = mime.as_ref().parse() {
                headers.insert(header::CONTENT_TYPE, content_type);
            }
            (StatusCode::OK, headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}
