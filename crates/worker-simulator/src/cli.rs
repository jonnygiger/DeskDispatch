use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "worker-simulator",
    about = "DeskDispatch Task Worker Simulator for realistic high-concurrency testing"
)]
pub struct SimCliArgs {
    /// DeskDispatch server base HTTP URL
    #[arg(long, default_value = "http://127.0.0.1:3000", env = "SERVER_URL")]
    pub server_url: String,

    /// Number of task workers to simulate concurrently
    #[arg(long, short = 'c', default_value_t = 10)]
    pub count: usize,

    /// Comma-separated list of pre-existing registration tokens
    #[arg(long)]
    pub registration_tokens: Option<String>,

    /// Pre-shared key for WPS unattended registration
    #[arg(long, env = "UNATTENDED_REGISTRATION_PSK")]
    pub psk: Option<String>,

    /// Comma-separated list of pre-existing API keys (dd_pk_...)
    #[arg(long)]
    pub api_keys: Option<String>,

    /// Optional max simulation run duration in seconds
    #[arg(long)]
    pub duration_secs: Option<u64>,

    /// Override worker profile (mixed, normal, flaky, power_outage, laggy_network, outdated_agent)
    #[arg(long, default_value = "mixed")]
    pub profile: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_default_args() {
        let args = SimCliArgs::parse_from(["worker-simulator"]);
        assert_eq!(args.server_url, "http://127.0.0.1:3000");
        assert_eq!(args.count, 10);
        assert_eq!(args.profile, "mixed");
        assert_eq!(args.registration_tokens, None);
    }

    #[test]
    fn test_cli_custom_args() {
        let args = SimCliArgs::parse_from([
            "worker-simulator",
            "--server-url",
            "http://localhost:8080",
            "--count",
            "100",
            "--profile",
            "flaky",
        ]);
        assert_eq!(args.server_url, "http://localhost:8080");
        assert_eq!(args.count, 100);
        assert_eq!(args.profile, "flaky");
    }
}
