use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

pub const ERR_USER_RATE_LIMIT: &str = "Too many failed login attempts. Please try again later.";
pub const ERR_IP_RATE_LIMIT: &str =
    "Too many failed login attempts from this IP address. Please try again later.";

const EVICTION_THRESHOLD: usize = 256;

#[derive(Debug)]
struct RateLimiterInner {
    user_attempts: HashMap<(IpAddr, String), Vec<Instant>>,
    ip_attempts: HashMap<IpAddr, Vec<Instant>>,
}

impl RateLimiterInner {
    fn prune_and_clean_user(
        &mut self,
        ip: IpAddr,
        username: &str,
        now: Instant,
        window: Duration,
    ) -> Option<usize> {
        match self.user_attempts.entry((ip, username.to_lowercase())) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let ts = entry.get_mut();
                ts.retain(|t| now.duration_since(*t) < window);
                if ts.is_empty() {
                    entry.remove();
                    None
                } else {
                    Some(ts.len())
                }
            }
            std::collections::hash_map::Entry::Vacant(_) => None,
        }
    }

    fn prune_and_clean_ip(&mut self, ip: IpAddr, now: Instant, window: Duration) -> Option<usize> {
        match self.ip_attempts.entry(ip) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let ts = entry.get_mut();
                ts.retain(|t| now.duration_since(*t) < window);
                if ts.is_empty() {
                    entry.remove();
                    None
                } else {
                    Some(ts.len())
                }
            }
            std::collections::hash_map::Entry::Vacant(_) => None,
        }
    }

    fn sweep_expired(&mut self, now: Instant, window: Duration) {
        self.user_attempts.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
        self.ip_attempts.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
    }
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

    pub fn check_rate_limit(&self, ip: IpAddr, username: &str) -> Result<(), &'static str> {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");

        if let Some(user_count) = inner.prune_and_clean_user(ip, username, now, self.window)
            && user_count >= self.max_user_attempts
        {
            return Err(ERR_USER_RATE_LIMIT);
        }

        if let Some(ip_count) = inner.prune_and_clean_ip(ip, now, self.window)
            && ip_count >= self.max_ip_attempts
        {
            return Err(ERR_IP_RATE_LIMIT);
        }

        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr, username: &str) {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");

        if inner.user_attempts.len() + inner.ip_attempts.len() > EVICTION_THRESHOLD {
            inner.sweep_expired(now, self.window);
        }

        let user_ts = inner
            .user_attempts
            .entry((ip, username.to_lowercase()))
            .or_default();
        user_ts.retain(|t| now.duration_since(*t) < self.window);
        user_ts.push(now);

        let ip_ts = inner.ip_attempts.entry(ip).or_default();
        ip_ts.retain(|t| now.duration_since(*t) < self.window);
        ip_ts.push(now);
    }

    pub fn clear(&self, ip: IpAddr, username: &str) {
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");
        inner.user_attempts.remove(&(ip, username.to_lowercase()));
    }

    pub fn entry_counts(&self) -> (usize, usize) {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");
        inner.sweep_expired(now, self.window);
        (inner.user_attempts.len(), inner.ip_attempts.len())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
