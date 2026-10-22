use worker_simulator::profile::WorkerProfile;
use worker_simulator::worker::{SimulatedWorker, WorkerConfig};

#[test]
fn test_simulator_worker_config_and_profiles() {
    let mut profile_counts = std::collections::HashMap::new();

    for idx in 0..100 {
        let profile = WorkerProfile::select_from_index(idx);
        *profile_counts.entry(profile).or_insert(0) += 1;

        let config = WorkerConfig {
            worker_index: idx + 1,
            hostname: format!("sim-worker-{:03}.local", idx + 1),
            display_name: format!("Simulated Worker {:03}", idx + 1),
            registration_token: Some(format!("dd_reg_test_{}", idx)),
            psk: if idx % 2 == 0 {
                Some("secret_psk_123".to_string())
            } else {
                None
            },
            api_key: None,
            profile,
            server_url: "http://127.0.0.1:3000".to_string(),
        };

        let worker = SimulatedWorker::new(config.clone());
        assert_eq!(worker.config.worker_index, idx + 1);
        assert!(worker.config.hostname.contains("sim-worker-"));
        assert_eq!(worker.config.profile, profile);

        // Verify agent version mapping for profile
        match profile {
            WorkerProfile::OutdatedAgent => {
                assert_eq!(profile.agent_version(), "0.0.1");
            }
            _ => {
                assert_eq!(profile.agent_version(), "1.0.0");
            }
        }

        // Verify OS info non-empty string generation
        assert!(!profile.os_info().is_empty());
    }

    // Verify all 5 profiles are represented across 100 indices
    assert_eq!(profile_counts.get(&WorkerProfile::Normal), Some(&60));
    assert_eq!(profile_counts.get(&WorkerProfile::Flaky), Some(&15));
    assert_eq!(profile_counts.get(&WorkerProfile::PowerOutage), Some(&10));
    assert_eq!(profile_counts.get(&WorkerProfile::LaggyNetwork), Some(&10));
    assert_eq!(profile_counts.get(&WorkerProfile::OutdatedAgent), Some(&5));
}
