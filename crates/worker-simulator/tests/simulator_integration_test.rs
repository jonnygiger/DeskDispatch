use worker_simulator::profile::WorkerProfile;
use worker_simulator::worker::{SimulatedWorker, WorkerConfig};

#[test]
fn test_simulator_worker_config_and_profiles() {
    for idx in 0..100 {
        let profile = WorkerProfile::select_from_index(idx);
        let config = WorkerConfig {
            worker_index: idx + 1,
            hostname: format!("sim-worker-{:03}.local", idx + 1),
            display_name: format!("Simulated Worker {:03}", idx + 1),
            registration_token: Some(format!("dd_reg_test_{}", idx)),
            api_key: None,
            profile,
            server_url: "http://127.0.0.1:3000".to_string(),
        };

        let worker = SimulatedWorker::new(config);
        assert_eq!(worker.config.worker_index, idx + 1);
        assert!(worker.config.hostname.contains("sim-worker-"));
    }
}
