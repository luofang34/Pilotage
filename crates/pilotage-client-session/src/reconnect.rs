//! The reconnect decision: bounded exponential backoff, observer-only.
//!
//! A transport loss is retryable by default; a typed permanent fault is
//! not. What recovery restores is observation alone: authority did not
//! survive the loss, so nothing here ever schedules a lease.

/// Backoff bounds for reconnect attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// Delay before the first retry, in milliseconds.
    pub initial_delay_ms: u64,
    /// Ceiling no delay exceeds, in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay_ms: 500,
            max_delay_ms: 15_000,
        }
    }
}

/// Attempt counter and delay computation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconnectState {
    attempts: u32,
}

impl ReconnectState {
    /// The next attempt's absolute instant, doubling per attempt up to the
    /// policy ceiling.
    pub(crate) fn next_attempt_at(&mut self, policy: &ReconnectPolicy, now_ms: u64) -> u64 {
        let exponent = self.attempts.min(16);
        self.attempts = self.attempts.wrapping_add(1);
        let delay = policy
            .initial_delay_ms
            .saturating_mul(1_u64 << exponent)
            .min(policy.max_delay_ms);
        now_ms.saturating_add(delay)
    }

    /// An admission proves the path works; the counter starts over.
    pub(crate) fn reset(&mut self) {
        self.attempts = 0;
    }
}
