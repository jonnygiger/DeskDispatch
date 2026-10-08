use deskdispatch::{
    auth::LoginRateLimiter,
    build_router,
    config::Config,
    routes::*,
    AppState,
};
use secrecy::ExposeSecret;
use argon2::{Argon2, PasswordHasher};
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::net::SocketAddr;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().expect("Failed to load configuration");

    init_tracing(&config);

    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str());

    match subcommand {
        Some("migrate") => {
            info!("Running database migrations...");
            let pool = connect_db(&config).await?;
            run_migrations(&pool).await?;
            info!("Database migrations completed successfully.");
            return Ok(());
        }
        Some("create-admin") => {
            info!("Creating or updating default admin user...");
            let pool = connect_db(&config).await?;
            create_admin_user(&pool).await?;
            info!("Admin user configuration completed successfully.");
            return Ok(());
        }
        Some("init-s3") => {
            info!("Ensuring S3 bucket exists...");
            let s3_client = build_s3_client(&config);
            ensure_s3_bucket(&s3_client, &config.s3_bucket).await?;
            info!("S3 bucket initialization completed successfully.");
            return Ok(());
        }
        Some("setup") => {
            info!("Running full setup (migrations, admin user, S3 bucket)...");
            let pool = connect_db(&config).await?;
            run_migrations(&pool).await?;
            create_admin_user(&pool).await?;
            let s3_client = build_s3_client(&config);
            ensure_s3_bucket(&s3_client, &config.s3_bucket).await?;
            info!("Full setup completed successfully.");
            return Ok(());
        }
        Some(cmd) => {
            error!("Unknown subcommand: {}", cmd);
            eprintln!("Usage: deskdispatch [migrate|create-admin|init-s3|setup]");
            std::process::exit(1);
        }
        None => {}
    }

    info!("Starting DeskDispatch task server...");

    let pool = connect_db(&config).await?;
    run_migrations(&pool).await?;
    let s3_client = build_s3_client(&config);

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool,
        s3_client,
        config: config.clone(),
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify.clone(),
    };

    // Spawn PgListener task for LISTEN/NOTIFY task_queue_changed
    let listener_db_url = config.database_url.clone();
    let listener_tx = tx_notify.clone();
    tokio::spawn(async move {
        loop {
            match sqlx::postgres::PgListener::connect(&listener_db_url).await {
                Ok(mut listener) => {
                    if let Err(e) = listener.listen("task_queue_changed").await {
                        error!("PgListener failed to listen on 'task_queue_changed': {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    info!("PgListener connected and listening on 'task_queue_changed'");
                    loop {
                        match listener.recv().await {
                            Ok(_notification) => {
                                let _ = listener_tx.send(());
                            }
                            Err(e) => {
                                error!("PgListener recv error: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("PgListener connection error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    });

    let db_pool = state.db.clone();
    let retention_s3_client = state.s3_client.clone();
    let retention_config = config.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        let mut retention_counter: u32 = 0;
        loop {
            interval.tick().await;
            if let Err(e) = sweep_stalled_task_runs(&db_pool).await {
                tracing::error!("Error sweeping stalled task runs: {}", e);
            }
            if let Err(e) = process_due_schedules(&db_pool).await {
                tracing::error!("Error processing due schedules: {}", e);
            }

            // Run retention jobs every 1 hour (120 * 30s)
            if retention_counter == 0 {
                deskdispatch::retention::run_all_retention_jobs(
                    &db_pool,
                    &retention_s3_client,
                    &retention_config,
                )
                .await;
            }
            retention_counter = (retention_counter + 1) % 120;
        }
    });

    let app = build_router(state);

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

async fn connect_db(config: &Config) -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(config.database_acquire_timeout_secs))
        .connect(&config.database_url)
        .await
        .map_err(|e| {
            error!("Failed to connect to Postgres: {}", e);
            e
        })?;
    Ok(pool)
}

async fn run_migrations(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| {
            error!("Failed to run database migrations: {}", e);
            e
        })?;
    Ok(())
}

async fn create_admin_user(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let admin_username = env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string());
    let admin_password = env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "test".to_string());
    let admin_display_name = env::var("ADMIN_DISPLAY_NAME").unwrap_or_else(|_| "Admin".to_string());

    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(admin_password.as_bytes())
        .map_err(|e| format!("Password hashing error: {}", e))?
        .to_string();

    sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active, must_change_password)
        VALUES ($1, $2, $3, 'admin', true, true)
        ON CONFLICT (username) DO UPDATE
        SET password_hash = EXCLUDED.password_hash,
            display_name = EXCLUDED.display_name,
            role = 'admin',
            is_active = true
        "#,
    )
    .bind(&admin_username)
    .bind(&password_hash)
    .bind(&admin_display_name)
    .execute(pool)
    .await?;

    info!("Seeded admin user '{}'", admin_username);
    Ok(())
}

fn build_s3_client(config: &Config) -> aws_sdk_s3::Client {
    let credentials = aws_sdk_s3::config::Credentials::new(
        &config.s3_access_key,
        config.s3_secret_key.expose_secret(),
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

    aws_sdk_s3::Client::from_conf(s3_config_builder.build())
}

async fn ensure_s3_bucket(
    s3_client: &aws_sdk_s3::Client,
    bucket_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match s3_client.head_bucket().bucket(bucket_name).send().await {
        Ok(_) => {
            info!("S3 bucket '{}' already exists.", bucket_name);
        }
        Err(_) => {
            info!("S3 bucket '{}' not found, creating...", bucket_name);
            s3_client
                .create_bucket()
                .bucket(bucket_name)
                .send()
                .await?;
            info!("S3 bucket '{}' created successfully.", bucket_name);
        }
    }

    // Set lifecycle rule to auto-expire tmp/ uploads after 1 day
    let filter = aws_sdk_s3::types::LifecycleRuleFilter::builder()
        .prefix("tmp/")
        .build();
    let rule = aws_sdk_s3::types::LifecycleRule::builder()
        .id("expire-tmp-uploads")
        .filter(filter)
        .status(aws_sdk_s3::types::ExpirationStatus::Enabled)
        .expiration(
            aws_sdk_s3::types::LifecycleExpiration::builder()
                .days(1)
                .build(),
        )
        .build();

    if let Ok(rule) = rule {
        let lifecycle_config = aws_sdk_s3::types::BucketLifecycleConfiguration::builder()
            .rules(rule)
            .build();

        if let Ok(lifecycle_config) = lifecycle_config {
            if let Err(e) = s3_client
                .put_bucket_lifecycle_configuration()
                .bucket(bucket_name)
                .lifecycle_configuration(lifecycle_config)
                .send()
                .await
            {
                tracing::warn!("Failed to set S3 bucket lifecycle policy (may be unsupported by mock/RustFS): {}", e);
            } else {
                info!("S3 lifecycle configuration set for 'tmp/' prefix on bucket '{}'", bucket_name);
            }
        }
    }

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

pub async fn livez_handler() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, "OK")
}

pub async fn readyz_handler(axum::extract::State(state): axum::extract::State<AppState>) -> impl axum::response::IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (axum::http::StatusCode::OK, "OK"),
        Err(err) => {
            tracing::error!("Readyz check failed: {}", err);
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Database connection error")
        }
    }
}
