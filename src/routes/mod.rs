pub mod account;
pub mod auth;
pub mod errors;
pub mod home;

pub use account::{get_password_handler, post_password_handler};
pub use auth::{get_login_handler, post_login_handler, post_logout_handler};
pub use errors::{
    internal_error_handler, not_found_handler, InternalServerErrorTemplate, NotFoundTemplate,
};
pub use home::get_index_handler;
