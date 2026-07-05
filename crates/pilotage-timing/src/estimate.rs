//! Round-trip latency estimation and cross-endpoint age estimation.
//!
//! ADR-0009 forbids comparing raw `MonoTimestamp` values across endpoints:
//! each endpoint's monotonic clock has its own epoch and drift. `RttEstimator`
//! and `ClockOffset` are the sanctioned path for turning a remote sample
//! timestamp into a locally meaningful age. Any code that instead subtracts a
//! remote `MonoTimestamp` from a local one directly is violating ADR-0009.

use core::time::Duration;

use crate::stamp::MonoTimestamp;

/// Exponentially-weighted moving average estimator for round-trip time.
///
/// Sans-IO: samples are supplied by the caller (`record`), never measured
/// internally. Smoothing factor is fixed to keep the estimator deterministic
/// and free of tuning knobs that would need their own justification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RttEstimator {
    estimate_nanos: Option<u64>,
}

/// Weight given to each new sample, as a fraction with denominator 8.
///
/// `alpha = 1/8` is the classic TCP RTT smoothing constant: a power-of-two
/// denominator keeps the update integer-exact (no floating point) and needs
/// only a handful of samples to converge within a few percent.
const ALPHA_NUMERATOR: u64 = 1;
const ALPHA_DENOMINATOR: u64 = 8;

impl RttEstimator {
    /// Constructs an estimator with no samples recorded yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            estimate_nanos: None,
        }
    }

    /// Folds one RTT sample into the running estimate.
    pub fn record(&mut self, rtt_sample: Duration) {
        let sample_nanos = u64::try_from(rtt_sample.as_nanos()).unwrap_or(u64::MAX);
        self.estimate_nanos = Some(match self.estimate_nanos {
            None => sample_nanos,
            Some(prev) => {
                // prev + alpha * (sample - prev), computed without floats.
                let weighted_prev = prev.saturating_mul(ALPHA_DENOMINATOR - ALPHA_NUMERATOR);
                let weighted_sample = sample_nanos.saturating_mul(ALPHA_NUMERATOR);
                weighted_prev.saturating_add(weighted_sample) / ALPHA_DENOMINATOR
            }
        });
    }

    /// Returns the current RTT estimate, or `None` if no sample was recorded.
    #[must_use]
    pub fn rtt(&self) -> Option<Duration> {
        self.estimate_nanos.map(Duration::from_nanos)
    }
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self::new()
    }
}

/// Signed nanosecond offset between a remote endpoint's clock and the local
/// clock: `local = remote + offset`.
///
/// Distinct from `MonoTimestamp` because it is a *difference* of two
/// endpoint-local epochs, not a point on either one. Estimating this offset
/// (e.g. via handshake round-trips) is out of scope for this crate; it is
/// consumed here as an explicit parameter, per ADR-0002's sans-IO discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockOffset {
    nanos: i64,
}

impl ClockOffset {
    /// Constructs an offset from a signed nanosecond count.
    #[must_use]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self { nanos }
    }

    /// Returns the offset as signed nanoseconds.
    #[must_use]
    pub const fn as_nanos(&self) -> i64 {
        self.nanos
    }

    /// Applies this offset to a remote timestamp, producing the equivalent
    /// point on the local clock, saturating at the `u64` nanosecond bounds
    /// instead of overflowing or panicking.
    #[must_use]
    pub fn translate_to_local(&self, remote: MonoTimestamp) -> MonoTimestamp {
        let remote_nanos = remote.as_nanos();
        let translated = if self.nanos >= 0 {
            #[allow(clippy::cast_sign_loss)]
            let offset = self.nanos as u64;
            remote_nanos.saturating_add(offset)
        } else {
            #[allow(clippy::cast_sign_loss)]
            let offset = self.nanos.unsigned_abs();
            remote_nanos.saturating_sub(offset)
        };
        MonoTimestamp::from_nanos(translated)
    }
}

/// Estimates how old a remote sample is, as observed on the local clock.
///
/// This is the only ADR-0009-sanctioned way to compare a remote
/// `MonoTimestamp` against a local one: the remote timestamp is first
/// translated into local-clock terms via `offset`, then compared with
/// saturating subtraction so clock skew or an out-of-order sample cannot
/// underflow into a bogus large age.
#[must_use]
pub fn estimated_age(
    local_receive: MonoTimestamp,
    remote_sample: MonoTimestamp,
    offset: ClockOffset,
) -> Duration {
    let translated = offset.translate_to_local(remote_sample);
    local_receive.saturating_duration_since(translated)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ClockOffset, Duration, MonoTimestamp, RttEstimator, estimated_age};

    #[test]
    fn rtt_estimator_starts_empty() {
        let estimator = RttEstimator::new();
        assert_eq!(estimator.rtt(), None);
    }

    #[test]
    fn rtt_estimator_converges_toward_steady_samples() {
        let mut estimator = RttEstimator::new();
        for _ in 0..200 {
            estimator.record(Duration::from_millis(40));
        }
        let rtt = estimator.rtt().expect("estimate present after samples");
        let diff_ms = rtt.as_millis().abs_diff(40);
        assert!(diff_ms <= 1, "expected convergence near 40ms, got {rtt:?}");
    }

    #[test]
    fn rtt_estimator_moves_toward_new_samples() {
        let mut estimator = RttEstimator::new();
        estimator.record(Duration::from_millis(100));
        let first = estimator.rtt().expect("estimate present");
        estimator.record(Duration::from_millis(0));
        let second = estimator.rtt().expect("estimate present");
        assert!(second < first);
    }

    #[test]
    fn estimated_age_is_zero_for_simultaneous_samples() {
        let local = MonoTimestamp::from_nanos(1_000);
        let remote = MonoTimestamp::from_nanos(1_000);
        let offset = ClockOffset::from_nanos(0);
        assert_eq!(
            estimated_age(local, remote, offset),
            Duration::from_nanos(0)
        );
    }

    #[test]
    fn estimated_age_saturates_when_remote_clock_skew_makes_it_appear_future() {
        // Remote clock runs far ahead: after translation the "remote" sample
        // lands after local receive time. Age must saturate to zero, not
        // underflow/panic.
        let local = MonoTimestamp::from_nanos(1_000);
        let remote = MonoTimestamp::from_nanos(1_000);
        let offset = ClockOffset::from_nanos(1_000_000); // remote appears far in the future
        assert_eq!(
            estimated_age(local, remote, offset),
            Duration::from_nanos(0)
        );
    }

    #[test]
    fn estimated_age_saturates_at_u64_bound_on_extreme_negative_offset() {
        let local = MonoTimestamp::from_nanos(u64::MAX);
        let remote = MonoTimestamp::from_nanos(0);
        let offset = ClockOffset::from_nanos(i64::MIN);
        // translate_to_local(0) with offset i64::MIN saturating_subs to 0;
        // age is then local - 0, the full span, and must not panic.
        let age = estimated_age(local, remote, offset);
        assert_eq!(age, Duration::from_nanos(u64::MAX));
    }

    #[test]
    fn clock_offset_translates_positive_and_negative() {
        let remote = MonoTimestamp::from_nanos(1_000);
        assert_eq!(
            ClockOffset::from_nanos(500)
                .translate_to_local(remote)
                .as_nanos(),
            1_500
        );
        assert_eq!(
            ClockOffset::from_nanos(-500)
                .translate_to_local(remote)
                .as_nanos(),
            500
        );
    }
}
