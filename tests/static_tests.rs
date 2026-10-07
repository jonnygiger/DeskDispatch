use app::routes::static_asset_handler;
use axum::{
    body::to_bytes,
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};

#[tokio::test]
async fn test_static_asset_style_css() {
    let headers = HeaderMap::new();
    let response = static_asset_handler(headers, Path("style.css".to_string()))
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

    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .expect("Cache-Control header present")
        .to_str()
        .unwrap();

    assert!(
        cache_control.contains("immutable"),
        "Expected immutable cache control, got {}",
        cache_control
    );

    assert!(
        response.headers().contains_key(header::ETAG),
        "Expected ETag header present"
    );

    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Read response body");

    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains(":root"), "CSS content expected");
}

#[tokio::test]
async fn test_static_asset_not_found() {
    let headers = HeaderMap::new();
    let response = static_asset_handler(headers, Path("nonexistent.css".to_string()))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_static_asset_etag_304() {
    let headers = HeaderMap::new();
    let response = static_asset_handler(headers, Path("style.css".to_string()))
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header present")
        .to_str()
        .unwrap()
        .to_string();

    let mut req_headers = HeaderMap::new();
    req_headers.insert(header::IF_NONE_MATCH, header::HeaderValue::from_str(&etag).unwrap());

    let not_modified_res = static_asset_handler(req_headers, Path("style.css".to_string()))
        .await
        .into_response();

    assert_eq!(not_modified_res.status(), StatusCode::NOT_MODIFIED);
}
