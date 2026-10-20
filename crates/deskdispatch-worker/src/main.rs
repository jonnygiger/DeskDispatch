use clap::Parser;
use deskdispatch_protocol::*;
use deskdispatch_worker::api_client::WorkerApiClient;
use deskdispatch_worker::config::{CliArgs, Commands, ServiceAction, WorkerConfig};
use deskdispatch_worker::runner::StepRunner;
use deskdispatch_worker::service::SystemdServiceManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&args.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!(
        version = deskdispatch_worker::version(),
        "Starting DeskDispatch Worker PC Client"
    );

    match args.command.clone().unwrap_or(Commands::Run) {
        Commands::Run => run_worker_loop(args).await?,
        Commands::ConfigGen { output } => {
            let config = WorkerConfig::default();
            config.save_to_file(&output)?;
            info!("Saved default worker configuration to {}", output.display());
        }
        Commands::Service { action } => handle_service_command(action)?,
    }

    Ok(())
}

async fn run_worker_loop(args: CliArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = WorkerConfig::load(&args)?;
    let mut api_client = WorkerApiClient::new(&config.server_url, config.api_key.clone());

    info!(
        server_url = %config.server_url,
        hostname = %config.hostname,
        display = %config.display,
        "Worker process initialized"
    );

    // Step 1: Register worker if no API key is set
    if api_client.api_key.is_none() {
        let mut registered = false;

        if let Some(ref psk) = config.psk {
            match api_client
                .register(None, Some(psk), &config.hostname, &config.display_name)
                .await
            {
                Ok(resp) => {
                    info!(
                        worker_id = resp.worker_id,
                        "Unattended worker registered successfully via WPS PSK"
                    );
                    api_client.set_api_key(resp.api_key.clone());
                    config.api_key = Some(resp.api_key);
                    registered = true;
                }
                Err(e) => {
                    warn!("WPS PSK registration failed: {}. Trying registration token...", e);
                }
            }
        }

        if !registered {
            if let Some(ref token) = config.registration_token {
                match api_client
                    .register(Some(token), None, &config.hostname, &config.display_name)
                    .await
                {
                    Ok(resp) => {
                        info!(
                            worker_id = resp.worker_id,
                            "Registered successfully with registration token"
                        );
                        api_client.set_api_key(resp.api_key.clone());
                        config.api_key = Some(resp.api_key);
                    }
                    Err(e) => {
                        return Err(format!("Registration with token failed: {}", e).into());
                    }
                }
            } else {
                return Err("Worker requires API key, WPS PSK, or registration token to start".into());
            }
        }

        if let Some(ref path) = args.config {
            let _ = config.save_to_file(path);
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received. Exiting worker loop...");
        r.store(false, Ordering::Relaxed);
    });

    // Step 2: Spawn background heartbeat loop
    let hb_client = api_client.clone();
    let hb_config = config.clone();
    let hb_running = running.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;

            if !hb_running.load(Ordering::Relaxed) {
                break;
            }

            let os_info = format!("Debian Linux x86_64 ({})", std::env::consts::ARCH);
            let agent_version = deskdispatch_worker::version();

            match hb_client
                .send_heartbeat(
                    "online",
                    None,
                    hb_config.screen_width,
                    hb_config.screen_height,
                    &os_info,
                    agent_version,
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    warn!("Heartbeat error: {}", e);
                }
            }
        }
    });

    let mut runner = StepRunner::new(config.clone(), api_client.clone());

    // Step 3: Main polling loop
    while running.load(Ordering::Relaxed) {
        match api_client.poll_next_assignment().await {
            Ok(NextAssignmentResponse::None) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(NextAssignmentResponse::ExecuteAutomation {
                task_run_id,
                automation,
            }) => {
                info!(task_run_id = task_run_id, "Executing assigned automation task");
                if let Err(e) = runner.execute_automation(task_run_id, &automation).await {
                    error!(
                        task_run_id = task_run_id,
                        "Automation execution error: {}", e
                    );
                }
            }
            Ok(NextAssignmentResponse::StartRecording {
                recording_session_id,
            }) => {
                info!(
                    recording_session_id = recording_session_id,
                    "Start recording assignment received"
                );
            }
            Err(e) => {
                warn!("Polling next assignment returned error: {}. Retrying in 5s...", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }

    info!("Worker agent stopped gracefully");
    Ok(())
}

fn handle_service_command(action: ServiceAction) -> Result<(), Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()?;

    match action {
        ServiceAction::Install { name, user } => {
            let manager = SystemdServiceManager::new(&name, &user);
            manager.install(&current_exe, None)?;
        }
        ServiceAction::Uninstall { name } => {
            let manager = SystemdServiceManager::new(&name, "root");
            manager.uninstall()?;
        }
        ServiceAction::Status { name } => {
            let manager = SystemdServiceManager::new(&name, "root");
            let output = manager.status()?;
            println!("{}", output);
        }
    }

    Ok(())
}
