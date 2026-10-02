pub mod account;
pub mod api_workers;
pub mod auth;
pub mod automations;
pub mod bitmaps;
pub mod errors;
pub mod home;
pub mod media;
pub mod static_assets;
pub mod workers;

pub use account::{get_password_handler, post_password_handler};
pub use api_workers::{get_next_assignment_handler, post_heartbeat_handler, post_register_worker_handler};
pub use auth::{get_login_handler, post_login_handler, post_logout_handler};
pub use automations::*;
pub use bitmaps::*;
pub use errors::{
    internal_error_handler, not_found_handler, InternalServerErrorTemplate, NotFoundTemplate,
};
pub use home::get_index_handler;
pub use media::*;
pub use static_assets::static_asset_handler;
pub use workers::*;
