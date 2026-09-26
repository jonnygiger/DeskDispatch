mod config;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use config::Config;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::net::SocketAddr;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub s3_client: aws_sdk_s3::Client,
    pub config: Config,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().expect("Failed to load configuration");

    init_tracing(&config);

    info!("Starting DeskDispatch task server...");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .map_err(|e| {
            error!("Failed to connect to Postgres: {}", e);
            e
        })?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            error!("Failed to run database migrations: {}", e);
            e
        })?;

    let credentials = aws_sdk_s3::config::Credentials::new(
        &config.s3_access_key,
        &config.s3_secret_key,
        None,
        None,
        "static",
    );

    let mut s3_config_builder = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_sdk_s3::config::Region::new(config.s3_region.clone()));

    if let Some(endpoint) = &config.s3_endpoint {
        s3_config_builder = s3_config_builder.endpoint_url(endpoint).force_path_style(true);
    }

    let s3_client = aws_sdk_s3::Client::from_conf(s3_config_builder.build());

    let state = AppState {
        db: pool,
        s3_client,
        config: config.clone(),
    };

    let app = Router::new()
        .route("/healthz", get(healthz_handler))
        .with_state(state);

    let addr: SocketAddr = config.bind_address.parse().expect("Invalid BIND_ADDRESS");
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn init_tracing(config: &Config) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,app=debug"));

    if config.is_production() {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().pretty())
            .init();
    }
}

async fn healthz_handler(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, "OK"),
        Err(err) => {
            error!("Healthz check failed: {}", err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database connection error")
        }
    }
}
