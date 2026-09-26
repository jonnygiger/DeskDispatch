use app::{
    auth::{csrf_middleware, LoginRateLimiter},
    config::Config,
    routes::{
        get_index_handler, get_login_handler, get_password_handler, not_found_handler,
        post_login_handler, post_logout_handler, post_password_handler, static_asset_handler,
    },
    AppState,
};
use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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
        rate_limiter: LoginRateLimiter::default(),
    };

    let app = Router::new()
        .route("/healthz", get(healthz_handler))
        .route("/static/{*path}", get(static_asset_handler))
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/logout", post(post_logout_handler))
        .route(
            "/account/password",
            get(get_password_handler).post(post_password_handler),
        )
        .route("/", get(get_index_handler))
        .fallback(not_found_handler)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf_middleware,
        ))
        .with_state(state);

    let addr: SocketAddr = config.bind_address.parse().expect("Invalid BIND_ADDRESS");
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

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
