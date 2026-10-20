use std::borrow::Cow;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

pub const ERR_USER_RATE_LIMIT: &str = "Too many failed login attempts. Please try again later.";
pub const ERR_IP_RATE_LIMIT: &str =
    "Too many failed login attempts from this IP address. Please try again later.";

const EVICTION_THRESHOLD: usize = 256;

#[inline]
fn normalize_username(username: &str) -> Cow<'_, str> {
    if username.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(username.to_ascii_lowercase())
    } else {
        Cow::Borrowed(username)
    }
}

#[derive(Debug)]
struct RateLimiterInner {
    user_attempts: HashMap<IpAddr, HashMap<String, Vec<Instant>>>,
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
        let norm = normalize_username(username);
        let (count, is_empty) = {
            let user_map = self.user_attempts.get_mut(&ip)?;
            let ts = user_map.get_mut(norm.as_ref())?;
            ts.retain(|t| now.duration_since(*t) < window);
            if ts.is_empty() {
                user_map.remove(norm.as_ref());
                (None, user_map.is_empty())
            } else {
                (Some(ts.len()), false)
            }
        };

        if is_empty {
            self.user_attempts.remove(&ip);
        }

        count
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
        self.user_attempts.retain(|_, user_map| {
            user_map.retain(|_, timestamps| {
                timestamps.retain(|t| now.duration_since(*t) < window);
                !timestamps.is_empty()
            });
            !user_map.is_empty()
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
        self.check_rate_limit_at(ip, username, Instant::now())
    }

    pub fn check_rate_limit_at(
        &self,
        ip: IpAddr,
        username: &str,
        now: Instant,
    ) -> Result<(), &'static str> {
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
        self.record_failure_at(ip, username, Instant::now());
    }

    pub fn record_failure_at(&self, ip: IpAddr, username: &str, now: Instant) {
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");

        if inner.user_attempts.len() + inner.ip_attempts.len() > EVICTION_THRESHOLD {
            inner.sweep_expired(now, self.window);
        }

        let norm = normalize_username(username);
        let user_map = inner.user_attempts.entry(ip).or_default();
        let user_ts = match user_map.get_mut(norm.as_ref()) {
            Some(ts) => ts,
            None => user_map.entry(norm.into_owned()).or_default(),
        };
        user_ts.retain(|t| now.duration_since(*t) < self.window);
        user_ts.push(now);

        let ip_ts = inner.ip_attempts.entry(ip).or_default();
        ip_ts.retain(|t| now.duration_since(*t) < self.window);
        ip_ts.push(now);
    }

    pub fn clear(&self, ip: IpAddr, username: &str) {
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");
        let norm = normalize_username(username);
        let mut is_empty = false;
        if let Some(user_map) = inner.user_attempts.get_mut(&ip) {
            user_map.remove(norm.as_ref());
            is_empty = user_map.is_empty();
        }
        if is_empty {
            inner.user_attempts.remove(&ip);
        }
    }

    pub fn entry_counts(&self) -> (usize, usize) {
        self.entry_counts_at(Instant::now())
    }

    pub fn entry_counts_at(&self, now: Instant) -> (usize, usize) {
        let mut inner = self.inner.lock().expect("lock rate limiter mutex");
        inner.sweep_expired(now, self.window);
        let total_user_entries = inner.user_attempts.values().map(|m| m.len()).sum();
        (total_user_entries, inner.ip_attempts.len())
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
        let t0 = Instant::now();

        // Check initially allowed
        assert!(limiter.check_rate_limit_at(ip, "user1", t0).is_ok());

        // 2 failures for user1 -> user1 limit reached
        limiter.record_failure_at(ip, "user1", t0);
        limiter.record_failure_at(ip, "user1", t0);

        assert!(limiter.check_rate_limit_at(ip, "user1", t0).is_err());
        // user2 on same IP is still under user limit, but total IP attempts is 2 (< 4)
        assert!(limiter.check_rate_limit_at(ip, "user2", t0).is_ok());

        // Add 2 failures for user2 -> total IP attempts becomes 4
        limiter.record_failure_at(ip, "user2", t0);
        limiter.record_failure_at(ip, "user2", t0);

        // Now user3 on same IP is blocked by per-IP limit!
        let err = limiter.check_rate_limit_at(ip, "user3", t0).unwrap_err();
        assert!(err.contains("from this IP address"));

        // Simulate window expiration without sleeping (60ms > 50ms window)
        let t1 = t0 + Duration::from_millis(60);

        // After window expiration, check_rate_limit_at evicts old entries
        assert!(limiter.check_rate_limit_at(ip, "user1", t1).is_ok());
        assert!(limiter.check_rate_limit_at(ip, "user3", t1).is_ok());

        let (user_count, ip_count) = limiter.entry_counts_at(t1);
        assert_eq!(user_count, 0);
        assert_eq!(ip_count, 0);
    }

    #[test]
    fn test_login_rate_limiter_case_insensitivity_clear_and_eviction() {
        let limiter = LoginRateLimiter::new_with_ip_limit(2, 100, Duration::from_millis(50));
        let ip1: IpAddr = "10.0.0.1".parse().unwrap();
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();
        let t0 = Instant::now();

        // 1. Case-insensitivity: "Admin", "ADMIN", "admin" share same bucket
        limiter.record_failure_at(ip1, "Admin", t0);
        limiter.record_failure_at(ip1, "ADMIN", t0);
        let err = limiter.check_rate_limit_at(ip1, "admin", t0).unwrap_err();
        assert_eq!(err, ERR_USER_RATE_LIMIT);

        // 2. Clear user attempts for ip1 and "admin"
        limiter.clear(ip1, "ADMIN");
        assert!(limiter.check_rate_limit_at(ip1, "admin", t0).is_ok());

        // 3. Eviction threshold: Fill past EVICTION_THRESHOLD (256) with expired entries
        let fast_limiter = LoginRateLimiter::new_with_ip_limit(10, 10, Duration::from_millis(10));
        for i in 0..150 {
            let ip: IpAddr = format!("172.16.{}.{}", i / 250, i % 250).parse().unwrap();
            fast_limiter.record_failure_at(ip, &format!("user{}", i), t0);
        }
        // Total entries: 150 user + 150 ip = 300 entries > 256 threshold
        let (u_before, ip_before) = fast_limiter.entry_counts_at(t0);
        assert!(u_before > 0 && ip_before > 0);

        // Simulate time progression so entries expire without sleeping (15ms > 10ms window)
        let t1 = t0 + Duration::from_millis(15);

        // Next failure triggers sweep_expired inside record_failure because count > 256
        fast_limiter.record_failure_at(ip2, "fresh_user", t1);

        // The old 150 expired entries should have been swept, leaving only fresh_user
        let (u_after, ip_after) = fast_limiter.entry_counts_at(t1);
        assert_eq!(u_after, 1);
        assert_eq!(ip_after, 1);
    }
}
