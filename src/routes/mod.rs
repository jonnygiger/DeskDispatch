#![allow(ambiguous_glob_reexports)]

pub mod account;
pub mod api_workers;
pub mod auth;
pub mod automations;
pub mod bitmaps;
pub mod deletion_tests;
pub mod errors;
pub mod home;
pub mod media;
pub mod recordings;
pub mod runs;
pub mod schedules;
pub mod static_assets;
pub mod users;
pub mod workers;

pub use account::*;
pub use api_workers::*;
pub use auth::*;
pub use automations::*;
pub use bitmaps::*;
pub use errors::*;
pub use home::*;
pub use media::*;
pub use recordings::*;
pub use runs::*;
pub use schedules::*;
pub use static_assets::*;
pub use users::*;
pub use workers::*;

pub async fn livez_handler() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, "OK")
}

pub async fn readyz_handler(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> impl axum::response::IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (axum::http::StatusCode::OK, "OK"),
        Err(err) => {
            tracing::error!("Readyz check failed: {}", err);
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Database connection error",
            )
        }
    }
}
