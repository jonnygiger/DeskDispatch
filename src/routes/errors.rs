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
