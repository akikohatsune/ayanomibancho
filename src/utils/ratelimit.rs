use crate::config::RateLimitConfig;
use crate::utils::security::is_private_or_loopback_ip;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitTier {
    General,
    Bancho,
    Direct,
    Sensitive,
}

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    fill_rate_per_sec: f64,
    last_update: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, rate_per_sec: f64) -> Self {
        Self {
            tokens: capacity,
            max_tokens: capacity,
            fill_rate_per_sec: rate_per_sec,
            last_update: Instant::now(),
        }
    }

    fn try_acquire(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.fill_rate_per_sec).min(self.max_tokens);
        self.last_update = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn retry_after_seconds(&self) -> u64 {
        if self.fill_rate_per_sec <= 0.0 {
            return 60;
        }
        let needed = 1.0 - self.tokens;
        if needed <= 0.0 {
            0
        } else {
            (needed / self.fill_rate_per_sec).ceil() as u64
        }
    }
}

#[derive(Debug)]
struct ClientBuckets {
    general: TokenBucket,
    bancho: TokenBucket,
    direct: TokenBucket,
    sensitive: TokenBucket,
    last_seen: Instant,
}

impl ClientBuckets {
    fn new(config: &RateLimitConfig) -> Self {
        let gen_rpm = config.general_rpm as f64;
        let ban_rpm = config.bancho_rpm as f64;
        let dir_rpm = config.direct_rpm as f64;
        let sen_rpm = config.sensitive_rpm as f64;

        Self {
            general: TokenBucket::new(gen_rpm.max(1.0), gen_rpm / 60.0),
            bancho: TokenBucket::new(ban_rpm.max(1.0), ban_rpm / 60.0),
            direct: TokenBucket::new(dir_rpm.max(1.0), dir_rpm / 60.0),
            sensitive: TokenBucket::new(sen_rpm.max(1.0), sen_rpm / 60.0),
            last_seen: Instant::now(),
        }
    }

    fn get_bucket_mut(&mut self, tier: RateLimitTier) -> &mut TokenBucket {
        self.last_seen = Instant::now();
        match tier {
            RateLimitTier::General => &mut self.general,
            RateLimitTier::Bancho => &mut self.bancho,
            RateLimitTier::Direct => &mut self.direct,
            RateLimitTier::Sensitive => &mut self.sensitive,
        }
    }
}

pub struct RateLimiter {
    clients: Mutex<HashMap<IpAddr, ClientBuckets>>,
    total_blocked: AtomicU64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
            total_blocked: AtomicU64::new(0),
        }
    }

    /// Checks if a request is allowed. Returns Ok(()) if allowed, or Err(retry_after_sec) if rate limited.
    pub fn check(
        &self,
        ip: &IpAddr,
        tier: RateLimitTier,
        config: &RateLimitConfig,
    ) -> Result<(), u64> {
        if !config.enabled {
            return Ok(());
        }

        // Whitelist loopback / local LAN IPs if configured
        if config.whitelist_private && is_private_or_loopback_ip(ip) {
            return Ok(());
        }

        let mut map = self.clients.lock().unwrap();
        let buckets = map
            .entry(*ip)
            .or_insert_with(|| ClientBuckets::new(config));

        let bucket = buckets.get_bucket_mut(tier);
        if bucket.try_acquire() {
            Ok(())
        } else {
            self.total_blocked.fetch_add(1, Ordering::Relaxed);
            let retry = bucket.retry_after_seconds().max(1);
            Err(retry)
        }
    }

    /// Prunes IPs that haven't been seen for longer than `max_idle`
    pub fn prune_stale(&self, max_idle: Duration) -> usize {
        let mut map = self.clients.lock().unwrap();
        let now = Instant::now();
        let initial_len = map.len();
        map.retain(|_, b| now.duration_since(b.last_seen) < max_idle);
        initial_len - map.len()
    }

    /// Total number of unique IPs currently tracked in memory
    pub fn tracked_ips_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    /// Total number of requests blocked across all IPs
    pub fn total_blocked_count(&self) -> u64 {
        self.total_blocked.load(Ordering::Relaxed)
    }
}

/// Classifies a request path and method into the corresponding RateLimitTier
pub fn classify_tier(method: &str, path: &str) -> RateLimitTier {
    // 1. Sensitive tier: Account creation, score submission, background uploads
    if path == "/api/register"
        || path == "/api/backgrounds/upload"
        || path == "/web/osu-submit-modular-selector.php"
    {
        return RateLimitTier::Sensitive;
    }

    // 2. Direct tier: osu!Direct beatmap search & download
    if path == "/web/osu-search.php"
        || path == "/web/osu-search-set.php"
        || path.starts_with("/d/")
    {
        return RateLimitTier::Direct;
    }

    // 3. Bancho tier: Bancho client packets & handshake
    if method == "POST" && (path == "/" || path == "/c") {
        return RateLimitTier::Bancho;
    }

    // 4. General tier: Web dashboard, avatars, matches, status, backgrounds, etc.
    RateLimitTier::General
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_tier_classification() {
        assert_eq!(classify_tier("POST", "/api/register"), RateLimitTier::Sensitive);
        assert_eq!(classify_tier("POST", "/web/osu-submit-modular-selector.php"), RateLimitTier::Sensitive);
        assert_eq!(classify_tier("POST", "/api/backgrounds/upload"), RateLimitTier::Sensitive);

        assert_eq!(classify_tier("GET", "/web/osu-search.php"), RateLimitTier::Direct);
        assert_eq!(classify_tier("GET", "/d/12345"), RateLimitTier::Direct);

        assert_eq!(classify_tier("POST", "/"), RateLimitTier::Bancho);
        assert_eq!(classify_tier("POST", "/c"), RateLimitTier::Bancho);

        assert_eq!(classify_tier("GET", "/"), RateLimitTier::General);
        assert_eq!(classify_tier("GET", "/api/status"), RateLimitTier::General);
        assert_eq!(classify_tier("GET", "/a/1"), RateLimitTier::General);
    }

    #[test]
    fn test_token_bucket_rate_limiting() {
        let config = RateLimitConfig {
            enabled: true,
            general_rpm: 60,
            bancho_rpm: 60,
            direct_rpm: 60,
            sensitive_rpm: 3, // only 3 tokens max
            whitelist_private: false, // ensure testing loopback is rate limited
        };

        let limiter = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 195));

        // 3 sensitive requests should succeed
        assert!(limiter.check(&ip, RateLimitTier::Sensitive, &config).is_ok());
        assert!(limiter.check(&ip, RateLimitTier::Sensitive, &config).is_ok());
        assert!(limiter.check(&ip, RateLimitTier::Sensitive, &config).is_ok());

        // 4th request must be rate limited
        let res = limiter.check(&ip, RateLimitTier::Sensitive, &config);
        assert!(res.is_err());
        assert_eq!(limiter.total_blocked_count(), 1);

        // However, a General tier request from the same IP is still allowed!
        assert!(limiter.check(&ip, RateLimitTier::General, &config).is_ok());
    }

    #[test]
    fn test_whitelist_private_ip() {
        let config = RateLimitConfig {
            enabled: true,
            general_rpm: 1,
            bancho_rpm: 1,
            direct_rpm: 1,
            sensitive_rpm: 1,
            whitelist_private: true,
        };

        let limiter = RateLimiter::new();
        let localhost = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Localhost should never be blocked when whitelist_private is true
        for _ in 0..50 {
            assert!(limiter.check(&localhost, RateLimitTier::Sensitive, &config).is_ok());
        }
        assert_eq!(limiter.total_blocked_count(), 0);
    }
}
