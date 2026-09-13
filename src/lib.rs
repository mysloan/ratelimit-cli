//! A token bucket rate limiter keyed by an arbitrary string (client id, IP, route, ...).
//!
//! Each key gets its own bucket that fills at `refill_per_sec` tokens per second up to
//! `capacity`. A check consumes one token if one is available.

use std::collections::HashMap;

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
}
