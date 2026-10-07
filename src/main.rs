use app::{
    auth::{csrf_middleware, security_headers_middleware, LoginRateLimiter},
    config::Config,
    routes::*,
    AppState,
};
use secrecy::ExposeSecret;
use argon2::{Argon2, PasswordHasher};
use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
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

    let state = AppState {
        db: pool,
        s3_client,
        config: config.clone(),
        rate_limiter: LoginRateLimiter::default(),
    };

    let db_pool = state.db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(e) = sweep_stalled_task_runs(&db_pool).await {
                tracing::error!("Error sweeping stalled task runs: {}", e);
            }
            if let Err(e) = process_due_schedules(&db_pool).await {
                tracing::error!("Error processing due schedules: {}", e);
            }
        }
    });

    let mut app = Router::new()
        .route("/livez", get(livez_handler))
        .route("/readyz", get(readyz_handler))
        .route("/healthz", get(readyz_handler))
        .route("/static/{*path}", get(static_asset_handler))
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/logout", post(post_logout_handler))
        .route(
            "/account/password",
            get(get_password_handler).post(post_password_handler),
        )
        .route("/", get(get_index_handler))
        .route("/automations", get(get_automations_handler).post(post_automations_handler))
        .route("/automations/new", get(get_new_automation_handler))
        .route("/automations/{id}", get(get_automation_detail_handler).post(post_automation_edit_handler))
        .route("/automations/{id}/run-now", post(post_run_now_automation_handler))
        .route("/automations/{id}/delete", get(get_automation_delete_handler).post(post_automation_delete_handler))
        .route("/automations/{id}/variables", get(get_automation_variables_handler).post(post_create_automation_variable_handler))
        .route("/automations/{id}/variables/{vid}", post(post_update_automation_variable_handler))
        .route("/automations/{id}/variables/{vid}/delete", post(post_delete_automation_variable_handler))
        .route("/automations/{id}/parameters", get(get_automation_parameters_handler).post(post_create_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}", post(post_update_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}/delete", post(post_delete_automation_parameter_handler))
        .route("/automations/{id}/steps/new", get(get_step_type_picker_handler))
        .route("/automations/{id}/steps/new/key_press", get(get_new_key_press_step_handler))
        .route("/automations/{id}/steps/new/mouse_click", get(get_new_mouse_click_step_handler))
        .route("/automations/{id}/steps/new/find_pixel_rgb", get(get_new_find_pixel_rgb_step_handler))
        .route("/automations/{id}/steps/new/find_bitmap", get(get_new_find_bitmap_step_handler))
        .route("/automations/{id}/steps/new/branch", get(get_new_branch_step_handler))
        .route("/automations/{id}/steps", post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", post(post_edit_step_handler))
        .route("/automations/{id}/steps/{sid}/move-up", post(post_move_step_up_handler))
        .route("/automations/{id}/steps/{sid}/move-down", post(post_move_step_down_handler))
        .route("/automations/{id}/steps/{sid}/delete", post(post_delete_step_handler))
        .route("/bitmaps", get(get_bitmaps_handler).post(post_bitmaps_handler))
        .route("/bitmaps/{id}/delete", post(post_delete_bitmap_handler))
        .route("/automations/{id}/bitmaps", get(get_automation_bitmaps_handler).post(post_automation_bitmaps_handler))
        .route("/automations/{id}/bitmaps/{bid}/delete", post(post_automation_delete_bitmap_handler))
        .route("/bitmaps/commit", get(get_bitmap_commit_handler).post(post_bitmap_commit_handler))
        .route("/bitmaps/pick-region", get(get_pick_region_handler).post(post_pick_region_top_left_handler))
        .route("/bitmaps/pick-region/bottom-right", post(post_pick_region_bottom_right_handler))
        .route("/bitmaps/pick-region/confirm", post(post_pick_region_confirm_handler))
        .route("/automations/{id}/bitmaps/pick-region", get(get_automation_pick_region_handler).post(post_automation_pick_region_top_left_handler))
        .route("/automations/{id}/bitmaps/pick-region/bottom-right", post(post_automation_pick_region_bottom_right_handler))
        .route("/automations/{id}/bitmaps/pick-region/confirm", post(post_automation_pick_region_confirm_handler))
        .route("/media/screenshots/{id}", get(get_media_screenshot_handler))
        .route("/media/bitmaps/{id}", get(get_media_bitmap_handler))
        .route("/schedules", get(get_schedules_handler).post(post_create_schedule_handler))
        .route("/schedules/new", get(get_new_schedule_handler))
        .route("/schedules/{id}/edit", get(get_edit_schedule_handler))
        .route("/schedules/{id}", post(post_edit_schedule_handler))
        .route("/schedules/{id}/toggle", post(post_toggle_schedule_handler))
        .route("/schedules/{id}/delete", post(post_delete_schedule_handler))
        .route("/api/v1/workers/register", post(post_register_worker_handler))
        .route("/api/v1/workers/heartbeat", post(post_heartbeat_handler))
        .route("/api/v1/workers/next-assignment", get(get_next_assignment_handler))
        .route("/api/v1/workers/task-runs/{id}", get(get_task_run_handler))
        .route("/api/v1/workers/task-runs/{id}/step-result", post(post_step_result_handler))
        .route("/api/v1/workers/task-runs/{id}/complete", post(post_complete_task_run_handler))
        .route("/api/v1/workers/task-runs/{id}/screenshot-upload-url", get(get_task_run_screenshot_upload_url_handler))
        .route("/api/v1/workers/task-runs/{id}/screenshots/commit", post(post_task_run_screenshot_commit_handler))
        .route("/api/v1/workers/recordings/{id}/events", post(post_recording_events_handler))
        .route("/api/v1/workers/recordings/{id}/screenshot-upload-url", get(get_recording_screenshot_upload_url_handler))
        .route("/api/v1/workers/recordings/{id}/stop", post(post_worker_stop_recording_handler))
        .route("/workers/{id}/record", get(get_worker_record_start_handler))
        .route("/workers/{id}/record/start", post(post_worker_record_start_handler))
        .route("/recordings/{id}", get(get_recording_status_handler))
        .route("/recordings/{id}/stop", post(post_recording_stop_handler))
        .route("/recordings/{id}/review", get(get_recording_review_handler))
        .route("/recordings/{id}/discard", post(post_recording_discard_handler))
        .route("/recordings/{id}/convert", post(post_recording_convert_handler))
        .route("/runs", get(get_runs_handler))
        .route("/runs/{id}", get(get_run_detail_handler))
        .route("/runs/{id}/cancel", post(post_cancel_run_handler))
        .route("/workers", get(get_workers_handler).post(post_create_worker_handler))
        .route("/workers/new", get(get_new_worker_handler))
        .route("/workers/{id}", get(get_worker_detail_handler))
        .route("/workers/{id}/edit", get(get_edit_worker_handler).post(post_edit_worker_handler))
        .route("/workers/{id}/delete", post(post_delete_worker_handler))
        .route("/workers/{id}/deactivate", post(post_deactivate_worker_handler))
        .route("/workers/{id}/rotate-key", post(post_rotate_worker_key_handler))
        .route("/worker-groups/new", get(get_new_worker_group_handler))
        .route("/worker-groups", post(post_create_worker_group_handler))
        .route("/worker-groups/{id}/edit", get(get_edit_worker_group_handler).post(post_edit_worker_group_handler))
        .route("/worker-groups/{id}/delete", post(post_delete_worker_group_handler));

    if !config.is_production() {
        app = app.route("/dev/magnifier-verify", get(dev_magnifier_verify_handler));
    }

    let app = app
        .fallback(not_found_handler)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers_middleware,
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

async fn connect_db(config: &Config) -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
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
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, $2, $3, 'admin', true)
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

async fn livez_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn readyz_handler(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, "OK"),
        Err(err) => {
            error!("Readyz check failed: {}", err);
            (StatusCode::SERVICE_UNAVAILABLE, "Database connection error")
        }
    }
}
