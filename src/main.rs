#![forbid(unsafe_code)]

use argon2::{Argon2, PasswordHasher};
use deskdispatch::{AppState, auth::LoginRateLimiter, build_router, config::Config, routes::*};
use secrecy::ExposeSecret;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::net::SocketAddr;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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
            info!("Running full setup (migrations, admin user, S3 bucket) with failsafes and retries...");
            let mut retries = 10;
            let pool = loop {
                match connect_db(&config).await {
                    Ok(p) => break p,
                    Err(e) if retries > 0 => {
                        error!("Database connection attempt failed during setup ({} retries left): {}. Sleeping 2s...", retries, e);
                        retries -= 1;
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => return Err(e),
                }
            };

            // Run migrations with retry failsafe
            let mut mig_retries = 5;
            loop {
                match run_migrations(&pool).await {
                    Ok(()) => break,
                    Err(e) if mig_retries > 0 => {
                        error!("Database migration attempt failed ({} retries left): {}. Sleeping 3s...", mig_retries, e);
                        mig_retries -= 1;
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                    Err(e) => return Err(e),
                }
            }

            // Pause briefly to ensure PostgreSQL transaction state and schema changes finalize
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            create_admin_user(&pool).await?;

            let s3_client = build_s3_client(&config);
            let mut s3_retries = 5;
            loop {
                match ensure_s3_bucket(&s3_client, &config.s3_bucket).await {
                    Ok(()) => break,
                    Err(e) if s3_retries > 0 => {
                        error!("S3 bucket init attempt failed ({} retries left): {}. Sleeping 2s...", s3_retries, e);
                        s3_retries -= 1;
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => return Err(e),
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            info!("Full setup completed successfully.");
            return Ok(());
        }
        Some("healthcheck") => {
            let addr: SocketAddr = config
                .bind_address
                .parse()
                .unwrap_or_else(|_| "127.0.0.1:3000".parse().expect("Valid socket address"));
            match check_health(&addr).await {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("Healthcheck failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(cmd) => {
            error!("Unknown subcommand: {}", cmd);
            eprintln!("Usage: deskdispatch [migrate|create-admin|init-s3|setup|healthcheck]");
            std::process::exit(1);
        }
        None => {}
    }

    info!("Starting DeskDispatch task server...");

    let pool = connect_db(&config).await?;
    run_migrations(&pool).await?;
    verify_database_schema(&pool).await?;
    let s3_client = build_s3_client(&config);

    let (tx_notify, _) = tokio::sync::broadcast::channel::<()>(100);

    let state = AppState {
        db: pool,
        s3_client,
        config: config.clone(),
        rate_limiter: LoginRateLimiter::default(),
        task_queue_notifier: tx_notify.clone(),
    };

    let cancel_token = tokio_util::sync::CancellationToken::new();
    let mut tasks = tokio::task::JoinSet::new();

    // Spawn PgListener task for LISTEN/NOTIFY task_queue_changed
    let listener_db_url = config.database_url.clone();
    let listener_tx = tx_notify.clone();
    let listener_cancel = cancel_token.clone();
    tasks.spawn(async move {
        loop {
            tokio::select! {
                _ = listener_cancel.cancelled() => {
                    info!("PgListener background task stopping...");
                    break;
                }
                conn_res = sqlx::postgres::PgListener::connect(&listener_db_url) => {
                    match conn_res {
                        Ok(mut listener) => {
                            if let Err(e) = listener.listen("task_queue_changed").await {
                                error!("PgListener failed to listen on 'task_queue_changed': {}", e);
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                continue;
                            }
                            info!("PgListener connected and listening on 'task_queue_changed'");
                            loop {
                                tokio::select! {
                                    _ = listener_cancel.cancelled() => break,
                                    recv_res = listener.recv() => {
                                        match recv_res {
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
                            }
                        }
                        Err(e) => {
                            error!("PgListener connection error: {}", e);
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                    }
                }
            }
        }
    });

    let db_pool = state.db.clone();
    let retention_s3_client = state.s3_client.clone();
    let retention_config = config.clone();
    let periodic_cancel = cancel_token.clone();
    tasks.spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut retention_counter: u32 = 0;
        loop {
            tokio::select! {
                _ = periodic_cancel.cancelled() => {
                    info!("Periodic background sweeper/scheduler task stopping...");
                    break;
                }
                _ = interval.tick() => {
                    let sweep_res = tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        sweep_stalled_task_runs(&db_pool)
                    ).await;
                    if let Ok(Err(e)) = sweep_res {
                        tracing::error!("Error sweeping stalled task runs: {}", e);
                    } else if sweep_res.is_err() {
                        tracing::error!("Timed out sweeping stalled task runs");
                    }

                    let sched_res = tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        process_due_schedules(&db_pool)
                    ).await;
                    if let Ok(Err(e)) = sched_res {
                        tracing::error!("Error processing due schedules: {}", e);
                    } else if sched_res.is_err() {
                        tracing::error!("Timed out processing due schedules");
                    }

                    // Run retention jobs every 1 hour (120 * 30s)
                    if retention_counter == 0 {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(60),
                            deskdispatch::retention::run_all_retention_jobs(
                                &db_pool,
                                &retention_s3_client,
                                &retention_config,
                            )
                        ).await;
                    }
                    retention_counter = (retention_counter + 1) % 120;
                }
            }
        }
    });

    // Optional separate Prometheus metrics listener
    if let Some(metrics_addr_str) = &config.metrics_bind_address
        && let Ok(metrics_addr) = metrics_addr_str.parse::<SocketAddr>()
    {
        let metrics_app =
            axum::Router::new().route("/metrics", axum::routing::get(metrics_handler));
        let metrics_cancel = cancel_token.clone();
        tasks.spawn(async move {
            if let Ok(metrics_listener) = tokio::net::TcpListener::bind(metrics_addr).await {
                info!("Metrics server listening on {}", metrics_addr);
                let _ = axum::serve(metrics_listener, metrics_app)
                    .with_graceful_shutdown(async move {
                        metrics_cancel.cancelled().await;
                    })
                    .await;
            }
        });
    }

    let app = build_router(state);

    let addr: SocketAddr = config.bind_address.parse().expect("Invalid BIND_ADDRESS");
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let shutdown_token = cancel_token.clone();

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        shutdown_token.cancel();
    })
    .await?;

    info!("Main server shutdown complete. Waiting for background tasks to finish...");
    while let Some(res) = tasks.join_next().await {
        if let Err(e) = res {
            tracing::warn!("Background task join error: {}", e);
        }
    }

    info!("All tasks stopped cleanly.");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
        },
        _ = terminate => {
            info!("Received SIGTERM, initiating graceful shutdown");
        },
    }
}

pub async fn metrics_handler() -> impl axum::response::IntoResponse {
    let metrics = "# HELP deskdispatch_up Process uptime status\n# TYPE deskdispatch_up gauge\ndeskdispatch_up 1\n";
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        metrics,
    )
}

async fn connect_db(config: &Config) -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(
            config.database_acquire_timeout_secs,
        ))
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

    verify_database_schema(pool).await?;
    Ok(())
}

async fn verify_database_schema(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    info!("Verifying required database constructs and schema failsafes...");

    let required_tables = [
        "users",
        "sessions",
        "audit_log",
        "automations",
        "automation_steps",
        "automation_variables",
        "automation_parameters",
        "bitmaps",
        "step_screenshots",
        "screenshots",
        "task_worker_pcs",
        "worker_groups",
        "worker_group_members",
        "schedules",
        "task_runs",
        "task_run_steps",
        "task_run_variable_values",
        "recording_sessions",
        "recording_events",
    ];

    for table in required_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(pool)
        .await?;

        if !exists {
            let err_msg = format!("Startup schema failsafe error: Required table '{}' is missing from the database.", table);
            error!("{}", err_msg);
            return Err(err_msg.into());
        }
    }

    let required_columns = [
        ("sessions", "last_active_at"),
        ("task_runs", "parameter_overrides"),
        ("task_runs", "cancel_requested_at"),
        ("task_runs", "target_worker_group_id"),
        ("task_runs", "dispatched_automation_json"),
        ("users", "must_change_password"),
    ];

    for (table, column) in required_columns {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2)",
        )
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await?;

        if !exists {
            let err_msg = format!("Startup schema failsafe error: Required column '{}.{}' is missing from the database.", table, column);
            error!("{}", err_msg);
            return Err(err_msg.into());
        }
    }

    info!("Database schema verification passed successfully.");
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
        s3_config_builder = s3_config_builder
            .endpoint_url(endpoint)
            .force_path_style(true);
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
            s3_client.create_bucket().bucket(bucket_name).send().await?;
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
                tracing::warn!(
                    "Failed to set S3 bucket lifecycle policy (may be unsupported by mock/RustFS): {}",
                    e
                );
            } else {
                info!(
                    "S3 lifecycle configuration set for 'tmp/' prefix on bucket '{}'",
                    bucket_name
                );
            }
        }
    }

    Ok(())
}

fn init_tracing(config: &Config) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

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

async fn check_health(addr: &SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(addr).await?;
    let request = format!(
        "GET /livez HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        addr
    );
    stream.write_all(request.as_bytes()).await?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).await?;
    let response = String::from_utf8_lossy(&buffer[..n]);

    if response.contains("200 OK") || response.contains("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(format!("Non-200 response: {}", response).into())
    }
}

pub async fn livez_handler() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, "OK")
}

pub async fn readyz_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
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
