use clap::Parser;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::task::JoinSet;
use tracing::{info, warn};

use worker_simulator::cli::SimCliArgs;
use worker_simulator::db_seeder::seed_workers_in_db;
use worker_simulator::profile::WorkerProfile;
use worker_simulator::worker::{SimulatedWorker, WorkerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = SimCliArgs::parse();

    info!(
        server_url = %args.server_url,
        count = args.count,
        profile = %args.profile,
        "Initializing DeskDispatch Task Worker Simulator"
    );

    let mut reg_tokens: Vec<Option<String>> = vec![None; args.count];
    let mut api_keys: Vec<Option<String>> = vec![None; args.count];

    if let Some(ref db_url) = args.database_url {
        info!("Seeding {} test workers in database...", args.count);
        match seed_workers_in_db(db_url, args.count).await {
            Ok(tokens) => {
                info!(
                    "Successfully seeded {} registration tokens in DB",
                    tokens.len()
                );
                for (i, tok) in tokens.into_iter().enumerate() {
                    if i < args.count {
                        reg_tokens[i] = Some(tok);
                    }
                }
            }
            Err(e) => {
                warn!("Database seeder error: {}. Proceeding without DB seed.", e);
            }
        }
    } else if let Some(ref tokens_str) = args.registration_tokens {
        let parsed_tokens: Vec<&str> = tokens_str.split(',').map(|s| s.trim()).collect();
        for (i, tok) in parsed_tokens.into_iter().enumerate() {
            if i < args.count && !tok.is_empty() {
                reg_tokens[i] = Some(tok.to_string());
            }
        }
    } else if let Some(ref keys_str) = args.api_keys {
        let parsed_keys: Vec<&str> = keys_str.split(',').map(|s| s.trim()).collect();
        for (i, key) in parsed_keys.into_iter().enumerate() {
            if i < args.count && !key.is_empty() {
                api_keys[i] = Some(key.to_string());
            }
        }
    }

    let stop_signal = Arc::new(AtomicBool::new(false));
    let mut tasks = JoinSet::new();

    for i in 0..args.count {
        let worker_idx = i + 1;
        let profile = match args.profile.to_lowercase().as_str() {
            "normal" => WorkerProfile::Normal,
            "flaky" => WorkerProfile::Flaky,
            "power_outage" => WorkerProfile::PowerOutage,
            "laggy_network" => WorkerProfile::LaggyNetwork,
            "outdated_agent" => WorkerProfile::OutdatedAgent,
            _ => WorkerProfile::select_from_index(i),
        };

        let config = WorkerConfig {
            worker_index: worker_idx,
            hostname: format!("sim-worker-{:03}.local", worker_idx),
            display_name: format!("Simulated Worker {:03}", worker_idx),
            registration_token: reg_tokens.get(i).cloned().flatten(),
            api_key: api_keys.get(i).cloned().flatten(),
            profile,
            server_url: args.server_url.clone(),
        };

        let worker = SimulatedWorker::new(config);
        let worker_stop = stop_signal.clone();

        tasks.spawn(async move {
            worker.run(worker_stop).await;
        });
    }

    info!(
        "Spawned {} simulated worker tasks successfully.",
        args.count
    );

    let ctrl_c = tokio::signal::ctrl_c();

    if let Some(secs) = args.duration_secs {
        info!("Simulator will run for duration of {} seconds...", secs);
        tokio::select! {
            _ = ctrl_c => {
                info!("Received Ctrl+C interrupt. Shutting down simulator...");
            }
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {
                info!("Duration timer expired ({}s). Shutting down simulator...", secs);
            }
        }
    } else {
        info!("Simulator running continuously. Press Ctrl+C to exit.");
        let _ = ctrl_c.await;
        info!("Received Ctrl+C interrupt. Shutting down simulator...");
    }

    stop_signal.store(true, Ordering::Relaxed);

    info!("Waiting for all simulated worker tasks to exit cleanly...");
    while let Some(res) = tasks.join_next().await {
        if let Err(e) = res {
            warn!("Worker task join error: {}", e);
        }
    }

    info!("Simulator shutdown complete.");
    Ok(())
}
