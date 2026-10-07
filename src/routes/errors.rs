use askama::Template;
use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::{AuthUser, OptionalAuthUser};
use crate::routes::auth::HtmlTemplate;

#[derive(Template)]
#[template(path = "404.html")]
pub struct NotFoundTemplate {
    pub user: Option<AuthUser>,
}

#[derive(Template)]
#[template(path = "403.html")]
pub struct ForbiddenTemplate {
    pub message: Option<String>,
    pub user: Option<AuthUser>,
}

#[derive(Template)]
#[template(path = "409.html")]
pub struct ConflictTemplate {
    pub message: Option<String>,
    pub user: Option<AuthUser>,
}

#[derive(Template)]
#[template(path = "500.html")]
pub struct InternalServerErrorTemplate {
    pub message: Option<String>,
    pub user: Option<AuthUser>,
}


#[tracing::instrument(skip(user))]
pub async fn not_found_handler(OptionalAuthUser(user): OptionalAuthUser) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        HtmlTemplate(NotFoundTemplate { user }),
    )
}

#[tracing::instrument(skip(user))]
pub async fn internal_error_handler(
    OptionalAuthUser(user): OptionalAuthUser,
    message: Option<String>,
) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        HtmlTemplate(InternalServerErrorTemplate { message, user }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn test_error_handlers_status_and_rendering() {
        // Test not_found_handler without user
        let response_404 = not_found_handler(OptionalAuthUser(None)).await.into_response();
        assert_eq!(response_404.status(), StatusCode::NOT_FOUND);

        let body_404 = axum::body::to_bytes(response_404.into_body(), usize::MAX)
            .await
            .unwrap();
        let html_404 = String::from_utf8(body_404.to_vec()).unwrap();
        assert!(html_404.contains("404") || html_404.contains("Not Found"));

        // Test internal_error_handler with error message
        let error_msg = "Database connection timed out".to_string();
        let response_500 = internal_error_handler(OptionalAuthUser(None), Some(error_msg.clone()))
            .await
            .into_response();
        assert_eq!(response_500.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body_500 = axum::body::to_bytes(response_500.into_body(), usize::MAX)
            .await
            .unwrap();
        let html_500 = String::from_utf8(body_500.to_vec()).unwrap();
        assert!(html_500.contains("Database connection timed out"));
    }
}
