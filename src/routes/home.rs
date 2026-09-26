use askama::Template;
use axum::response::IntoResponse;

use super::auth::HtmlTemplate;
use crate::auth::AuthUser;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub user: AuthUser,
}

pub async fn get_index_handler(user: AuthUser) -> impl IntoResponse {
    HtmlTemplate(IndexTemplate { user })
}
