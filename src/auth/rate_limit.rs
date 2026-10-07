use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug)]
struct RateLimiterInner {
    user_attempts: HashMap<(IpAddr, String), Vec<Instant>>,
    ip_attempts: HashMap<IpAddr, Vec<Instant>>,
}

#[derive(Clone, Debug)]
pub struct LoginRateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
    max_user_attempts: usize,
    max_ip_attempts: usize,
    window: Duration,
    pub argon2_semaphore: Arc<Semaphore>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self::new_with_ip_limit(5, 20, Duration::from_secs(15 * 60))
    }
}

impl LoginRateLimiter {
    pub fn new(max_user_attempts: usize, window: Duration) -> Self {
        Self::new_with_ip_limit(max_user_attempts, max_user_attempts * 4, window)
    }

    pub fn new_with_ip_limit(
        max_user_attempts: usize,
        max_ip_attempts: usize,
        window: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                user_attempts: HashMap::new(),
                ip_attempts: HashMap::new(),
            })),
            max_user_attempts,
            max_ip_attempts,
            window,
            argon2_semaphore: Arc::new(Semaphore::new(10)),
        }
    }

    fn sweep_expired(&self, inner: &mut RateLimiterInner, now: Instant) {
        let window = self.window;
        inner.user_attempts.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
        inner.ip_attempts.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
    }

    pub fn check_rate_limit(&self, ip: IpAddr, username: &str) -> Result<(), String> {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap();
        self.sweep_expired(&mut inner, now);

        let key = (ip, username.to_lowercase());

        if let Some(timestamps) = inner.user_attempts.get(&key) {
            if timestamps.len() >= self.max_user_attempts {
                return Err("Too many failed login attempts. Please try again later.".to_string());
            }
        }

        if let Some(ip_timestamps) = inner.ip_attempts.get(&ip) {
            if ip_timestamps.len() >= self.max_ip_attempts {
                return Err("Too many failed login attempts from this IP address. Please try again later.".to_string());
            }
        }

        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr, username: &str) {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap();
        self.sweep_expired(&mut inner, now);

        let key = (ip, username.to_lowercase());

        inner.user_attempts.entry(key).or_default().push(now);
        inner.ip_attempts.entry(ip).or_default().push(now);
    }

    pub fn clear(&self, ip: IpAddr, username: &str) {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap();
        let key = (ip, username.to_lowercase());
        inner.user_attempts.remove(&key);
        self.sweep_expired(&mut inner, now);
    }

    pub fn entry_counts(&self) -> (usize, usize) {
        let inner = self.inner.lock().unwrap();
        (inner.user_attempts.len(), inner.ip_attempts.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dual_rate_limiting_and_eviction() {
        let limiter = LoginRateLimiter::new_with_ip_limit(2, 4, Duration::from_millis(50));
        let ip: IpAddr = "192.168.1.50".parse().unwrap();

        // Check initially allowed
        assert!(limiter.check_rate_limit(ip, "user1").is_ok());

        // 2 failures for user1 -> user1 limit reached
        limiter.record_failure(ip, "user1");
        limiter.record_failure(ip, "user1");

        assert!(limiter.check_rate_limit(ip, "user1").is_err());
        // user2 on same IP is still under user limit, but total IP attempts is 2 (< 4)
        assert!(limiter.check_rate_limit(ip, "user2").is_ok());

        // Add 2 failures for user2 -> total IP attempts becomes 4
        limiter.record_failure(ip, "user2");
        limiter.record_failure(ip, "user2");

        // Now user3 on same IP is blocked by per-IP limit!
        let err = limiter.check_rate_limit(ip, "user3").unwrap_err();
        assert!(err.contains("from this IP address"));

        // Wait for window expiration
        std::thread::sleep(Duration::from_millis(60));

        // After window expiration, check_rate_limit evicts old entries
        assert!(limiter.check_rate_limit(ip, "user1").is_ok());
        assert!(limiter.check_rate_limit(ip, "user3").is_ok());

        let (user_count, ip_count) = limiter.entry_counts();
        assert_eq!(user_count, 0);
        assert_eq!(ip_count, 0);
    }
}
