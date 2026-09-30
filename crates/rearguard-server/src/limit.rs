// SPDX-License-Identifier: MIT OR Apache-2.0

//! Per-connection token-bucket rate limiting.

use std::time::Instant;

/// A token bucket: `rate` tokens per second, holding at most `burst`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub(crate) fn new(rate: f64, burst: f64, now: Instant) -> Self {
        Self {
            rate,
            burst,
            tokens: burst,
            last: now,
        }
    }

    /// Takes `n` tokens if available.
    pub(crate) fn take(&mut self, n: f64, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn burst_then_rate() {
        let t0 = Instant::now();
        let mut b = Bucket::new(10.0, 5.0, t0);
        for _ in 0..5 {
            assert!(b.take(1.0, t0));
        }
        assert!(!b.take(1.0, t0), "burst spent");
        assert!(
            b.take(1.0, t0 + Duration::from_millis(100)),
            "refilled one token"
        );
        assert!(!b.take(1.0, t0 + Duration::from_millis(100)));
        assert!(b.take(5.0, t0 + Duration::from_secs(10)), "capped at burst");
        assert!(
            !b.take(6.0, t0 + Duration::from_secs(100)),
            "never above burst"
        );
    }
}
