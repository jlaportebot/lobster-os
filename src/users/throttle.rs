//! Brute-force throttle for interactive logins: after
//! [`MAX_FAILURES`] wrong passwords in a row, further attempts are refused for
//! [`COOLDOWN_MS`]. Pure (time is passed in) so the lock screen and the greeter
//! share one tested state machine.

/// Consecutive wrong passwords before a cooldown starts.
pub const MAX_FAILURES: u32 = 5;
/// Length of the cooldown, in milliseconds.
pub const COOLDOWN_MS: u64 = 30_000;

/// What a failed attempt led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    /// Wrong password; this many tries remain before a cooldown.
    Retry(u32),
    /// That was the last allowed try: the cooldown has started.
    CooldownStarted,
}

/// Failure counter and cooldown deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Throttle {
    failures: u32,
    blocked_until_ms: u64,
}

impl Throttle {
    /// No failures recorded.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            failures: 0,
            blocked_until_ms: 0,
        }
    }

    /// Milliseconds of cooldown left at `now_ms` (0 when attempts are allowed).
    #[must_use]
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.blocked_until_ms.saturating_sub(now_ms)
    }

    /// True while attempts are refused.
    #[must_use]
    pub fn is_blocked(&self, now_ms: u64) -> bool {
        self.remaining_ms(now_ms) > 0
    }

    /// Record a wrong password at `now_ms`.
    pub fn record_failure(&mut self, now_ms: u64) -> Strike {
        self.failures += 1;
        if self.failures < MAX_FAILURES {
            return Strike::Retry(MAX_FAILURES - self.failures);
        }
        self.failures = 0;
        self.blocked_until_ms = now_ms + COOLDOWN_MS;
        Strike::CooldownStarted
    }

    /// Forget past failures after a correct password.
    pub fn record_success(&mut self) {
        *self = Self::new();
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_failures_still_allow_a_fifth_try() {
        let mut t = Throttle::new();
        for left in (1..MAX_FAILURES).rev() {
            assert_eq!(t.record_failure(0), Strike::Retry(left));
        }
        assert!(!t.is_blocked(0));
    }

    #[test]
    fn fifth_failure_starts_the_cooldown() {
        let mut t = Throttle::new();
        for _ in 1..MAX_FAILURES {
            t.record_failure(1_000);
        }
        assert_eq!(t.record_failure(1_000), Strike::CooldownStarted);
        assert_eq!(t.remaining_ms(1_000), COOLDOWN_MS);
        assert_eq!(t.remaining_ms(1_000 + 10_000), COOLDOWN_MS - 10_000);
        assert!(t.is_blocked(1_000 + COOLDOWN_MS - 1));
    }

    #[test]
    fn cooldown_expires_and_counting_restarts() {
        let mut t = Throttle::new();
        for _ in 0..MAX_FAILURES {
            t.record_failure(0);
        }
        assert!(!t.is_blocked(COOLDOWN_MS));
        assert_eq!(
            t.record_failure(COOLDOWN_MS),
            Strike::Retry(MAX_FAILURES - 1)
        );
    }

    #[test]
    fn success_clears_failures() {
        let mut t = Throttle::new();
        t.record_failure(0);
        t.record_failure(0);
        t.record_success();
        assert_eq!(t, Throttle::new());
    }
}
