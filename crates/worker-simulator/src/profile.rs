use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkerProfile {
    /// Standard well-behaved worker executing tasks successfully and sending timely heartbeats.
    Normal,
    /// Worker that generates random/flaky data, unexpected variable values, or occasional step failures.
    Flaky,
    /// Worker that abruptly stops responding mid-task or mid-heartbeat (simulating power outage / crash).
    PowerOutage,
    /// Worker with high network latency and delayed heartbeats/polling.
    LaggyNetwork,
    /// Worker with an outdated agent version that triggers 426 Upgrade Required.
    OutdatedAgent,
}

impl WorkerProfile {
    pub fn select_random() -> Self {
        let mut rng = rand::thread_rng();
        let roll: u32 = rng.gen_range(0..100);
        match roll {
            0..60 => WorkerProfile::Normal,
            60..75 => WorkerProfile::Flaky,
            75..85 => WorkerProfile::PowerOutage,
            85..95 => WorkerProfile::LaggyNetwork,
            _ => WorkerProfile::OutdatedAgent,
        }
    }

    pub fn select_from_index(index: usize) -> Self {
        match index % 20 {
            0..12 => WorkerProfile::Normal,
            12..15 => WorkerProfile::Flaky,
            15..17 => WorkerProfile::PowerOutage,
            17..19 => WorkerProfile::LaggyNetwork,
            _ => WorkerProfile::OutdatedAgent,
        }
    }

    pub fn agent_version(&self) -> &'static str {
        match self {
            WorkerProfile::OutdatedAgent => "0.0.1",
            _ => "1.0.0",
        }
    }

    pub fn os_info(&self) -> String {
        let os_list = [
            "Windows 11 Pro 23H2 x86_64",
            "Ubuntu 24.04 LTS x86_64",
            "macOS 14.5 Sonoma arm64",
            "Windows 10 Enterprise x86_64",
            "Debian 12 Bookworm x86_64",
        ];
        let mut rng = rand::thread_rng();
        os_list[rng.gen_range(0..os_list.len())].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_selection_index() {
        assert_eq!(WorkerProfile::select_from_index(0), WorkerProfile::Normal);
        assert_eq!(WorkerProfile::select_from_index(13), WorkerProfile::Flaky);
        assert_eq!(
            WorkerProfile::select_from_index(16),
            WorkerProfile::PowerOutage
        );
        assert_eq!(
            WorkerProfile::select_from_index(18),
            WorkerProfile::LaggyNetwork
        );
        assert_eq!(
            WorkerProfile::select_from_index(19),
            WorkerProfile::OutdatedAgent
        );
    }

    #[test]
    fn test_agent_version() {
        assert_eq!(WorkerProfile::OutdatedAgent.agent_version(), "0.0.1");
        assert_eq!(WorkerProfile::Normal.agent_version(), "1.0.0");
    }
}
