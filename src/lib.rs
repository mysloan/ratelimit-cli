//! Rate limiting algorithms keyed by an arbitrary string (client id, IP, route, ...).
//!
//! [`TokenBucketLimiter`] fills a per-key bucket at a fixed rate up to a capacity.
//! [`SlidingWindowLimiter`] instead counts requests in a trailing time window per key.
//! Both implement [`RateLimiter`] so callers (and the CLI) can pick one at runtime.

use std::collections::{HashMap, VecDeque};

/// Common interface for the rate limiting algorithms in this crate, so callers can
/// swap algorithms without changing the code that drives them.
pub trait RateLimiter {
    /// Returns true if a request for `key` at `now_ms` is allowed.
    fn check(&mut self, key: &str, now_ms: u64) -> bool;
}

struct Bucket {
    tokens: f64,
    last_update_ms: u64,
}

pub struct TokenBucketLimiter {
    capacity: f64,
    refill_per_ms: f64,
    buckets: HashMap<String, Bucket>,
}

impl TokenBucketLimiter {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        TokenBucketLimiter {
            capacity,
            refill_per_ms: refill_per_sec / 1000.0,
            buckets: HashMap::new(),
        }
    }

    /// Returns true if a request for `key` at `now_ms` is allowed, consuming a token.
    ///
    /// `now_ms` is caller-supplied (rather than read from the clock) so that a log of
    /// past events can be replayed deterministically.
    pub fn check(&mut self, key: &str, now_ms: u64) -> bool {
        let capacity = self.capacity;
        let refill_per_ms = self.refill_per_ms;
        let bucket = self.buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            last_update_ms: now_ms,
        });

        // Clamp to zero: an out-of-order timestamp (clock skew, unsorted input) must
        // not grant free tokens for negative elapsed time.
        let elapsed_ms = now_ms.saturating_sub(bucket.last_update_ms) as f64;
        bucket.tokens = (bucket.tokens + elapsed_ms * refill_per_ms).min(capacity);
        bucket.last_update_ms = now_ms;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Drops any bucket that has been full and idle since before `now_ms - max_age_ms`.
    /// Call this periodically in long-running processes so `buckets` doesn't grow
    /// without bound as new keys appear.
    pub fn evict_idle(&mut self, now_ms: u64, max_age_ms: u64) {
        self.buckets
            .retain(|_, b| now_ms.saturating_sub(b.last_update_ms) < max_age_ms);
    }
}

impl RateLimiter for TokenBucketLimiter {
    fn check(&mut self, key: &str, now_ms: u64) -> bool {
        self.check(key, now_ms)
    }
}

/// A sliding window log limiter: allows at most `limit` requests per key in any
/// trailing `window_ms` window, tracked by keeping each request's timestamp.
///
/// Unlike the token bucket, this looks at actual request history rather than an
/// averaged fill rate, so it never allows a burst larger than `limit` regardless of
/// how the requests inside the window are spaced.
pub struct SlidingWindowLimiter {
    limit: usize,
    window_ms: u64,
    history: HashMap<String, VecDeque<u64>>,
}

impl SlidingWindowLimiter {
    pub fn new(limit: usize, window_ms: u64) -> Self {
        SlidingWindowLimiter {
            limit,
            window_ms,
            history: HashMap::new(),
        }
    }

    /// Returns true if a request for `key` at `now_ms` is allowed, recording it.
    ///
    /// `now_ms` is caller-supplied for the same reason as `TokenBucketLimiter::check`:
    /// it lets a log of past events be replayed deterministically.
    pub fn check(&mut self, key: &str, now_ms: u64) -> bool {
        let window_ms = self.window_ms;
        let entry = self.history.entry(key.to_string()).or_default();

        // Drop timestamps that have aged out of the window: a request at `t` is
        // still counted while `t + window_ms > now_ms`. Comparing this way (instead
        // of subtracting window_ms from now_ms) avoids a saturating subtraction that
        // would otherwise clamp to zero and make every early timestamp look expired
        // at once. Filtering rather than popping from the front also tolerates
        // out-of-order input (clock skew, unsorted logs) without corrupting the deque.
        entry.retain(|&t| t.saturating_add(window_ms) > now_ms);

        if entry.len() < self.limit {
            entry.push_back(now_ms);
            true
        } else {
            false
        }
    }

    /// Drops any key whose most recent request was before `now_ms - max_age_ms`.
    /// Call this periodically in long-running processes so `history` doesn't grow
    /// without bound as new keys appear.
    pub fn evict_idle(&mut self, now_ms: u64, max_age_ms: u64) {
        self.history
            .retain(|_, h| h.back().is_some_and(|&t| now_ms.saturating_sub(t) < max_age_ms));
    }
}

impl RateLimiter for SlidingWindowLimiter {
    fn check(&mut self, key: &str, now_ms: u64) -> bool {
        self.check(key, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_capacity_then_blocks() {
        let mut limiter = TokenBucketLimiter::new(3.0, 1.0);
        assert!(limiter.check("a", 0));
        assert!(limiter.check("a", 0));
        assert!(limiter.check("a", 0));
        assert!(!limiter.check("a", 0));
    }

    #[test]
    fn refills_over_time() {
        let mut limiter = TokenBucketLimiter::new(1.0, 1.0);
        assert!(limiter.check("a", 0));
        assert!(!limiter.check("a", 500));
        assert!(limiter.check("a", 1000));
    }

    #[test]
    fn keys_are_independent() {
        let mut limiter = TokenBucketLimiter::new(1.0, 1.0);
        assert!(limiter.check("a", 0));
        assert!(limiter.check("b", 0));
    }

    #[test]
    fn out_of_order_timestamps_do_not_grant_free_tokens() {
        let mut limiter = TokenBucketLimiter::new(1.0, 1.0);
        assert!(limiter.check("a", 1000));
        assert!(!limiter.check("a", 500));
    }

    #[test]
    fn evict_idle_removes_stale_buckets() {
        let mut limiter = TokenBucketLimiter::new(1.0, 1.0);
        limiter.check("a", 0);
        limiter.evict_idle(10_000, 5_000);
        assert_eq!(limiter.buckets.len(), 0);
    }

    #[test]
    fn sliding_window_allows_up_to_limit_then_blocks() {
        let mut limiter = SlidingWindowLimiter::new(3, 1000);
        assert!(limiter.check("a", 0));
        assert!(limiter.check("a", 100));
        assert!(limiter.check("a", 200));
        assert!(!limiter.check("a", 300));
    }

    #[test]
    fn sliding_window_allows_again_once_oldest_request_ages_out() {
        let mut limiter = SlidingWindowLimiter::new(2, 1000);
        assert!(limiter.check("a", 0));
        assert!(limiter.check("a", 100));
        assert!(!limiter.check("a", 900));
        // the request at t=0 has now aged out of the [1-1000, 1000] window
        assert!(limiter.check("a", 1000));
    }

    #[test]
    fn sliding_window_keys_are_independent() {
        let mut limiter = SlidingWindowLimiter::new(1, 1000);
        assert!(limiter.check("a", 0));
        assert!(limiter.check("b", 0));
    }

    #[test]
    fn sliding_window_out_of_order_timestamp_is_still_counted() {
        let mut limiter = SlidingWindowLimiter::new(1, 1000);
        assert!(limiter.check("a", 1000));
        assert!(!limiter.check("a", 500));
    }

    #[test]
    fn sliding_window_evict_idle_removes_stale_keys() {
        let mut limiter = SlidingWindowLimiter::new(1, 1000);
        limiter.check("a", 0);
        limiter.evict_idle(10_000, 5_000);
        assert_eq!(limiter.history.len(), 0);
    }

    #[test]
    fn both_algorithms_implement_rate_limiter() {
        fn use_as_trait_object(limiter: &mut dyn RateLimiter, now_ms: u64) -> bool {
            limiter.check("a", now_ms)
        }

        let mut token_bucket = TokenBucketLimiter::new(1.0, 1.0);
        let mut sliding_window = SlidingWindowLimiter::new(1, 1000);
        assert!(use_as_trait_object(&mut token_bucket, 0));
        assert!(use_as_trait_object(&mut sliding_window, 0));
    }
}
