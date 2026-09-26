use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct LoginRateLimiter {
    attempts: Arc<Mutex<HashMap<(IpAddr, String), Vec<Instant>>>>,
    max_attempts: usize,
    window: Duration,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self::new(5, Duration::from_secs(15 * 60)) // 5 attempts per 15 minutes
    }
}

impl LoginRateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            max_attempts,
            window,
        }
    }

    pub fn check_rate_limit(&self, ip: IpAddr, username: &str) -> Result<(), String> {
        let now = Instant::now();
        let mut attempts = self.attempts.lock().unwrap();
        let key = (ip, username.to_lowercase());

        if let Some(timestamps) = attempts.get_mut(&key) {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            if timestamps.len() >= self.max_attempts {
                return Err("Too many failed login attempts. Please try again later.".to_string());
            }
        }

        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr, username: &str) {
        let now = Instant::now();
        let mut attempts = self.attempts.lock().unwrap();
        let key = (ip, username.to_lowercase());

        let timestamps = attempts.entry(key).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);
        timestamps.push(now);
    }

    pub fn clear(&self, ip: IpAddr, username: &str) {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.remove(&(ip, username.to_lowercase()));
    }
}
