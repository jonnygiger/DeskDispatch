pub mod api_client;
pub mod config;
pub mod engine;
pub mod runner;
pub mod service;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
