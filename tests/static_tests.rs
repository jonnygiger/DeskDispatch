use app::routes::static_asset_handler;
use axum::{
    body::to_bytes,
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
};

#[tokio::test]
async fn test_static_asset_style_css() {
    let response = static_asset_handler(Path("style.css".to_string()))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("Content-Type header present")
        .to_str()
        .unwrap();

    assert!(
        content_type.contains("text/css"),
        "Expected text/css, got {}",
        content_type
    );

    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Read response body");

    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains(":root"), "CSS content expected");
}

#[tokio::test]
async fn test_static_asset_not_found() {
    let response = static_asset_handler(Path("nonexistent.css".to_string()))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
