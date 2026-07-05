//! Monotonic timestamps and simulation ticks.
//!
//! Both types are opaque newtypes: they carry no notion of wall-clock time and
//! are only ever constructed from a value the caller already has in hand,
//! keeping this crate sans-IO per ADR-0002.

use core::time::Duration;

/// A monotonic instant expressed as nanoseconds since an endpoint-local epoch.
///
/// `MonoTimestamp` corresponds to ADR-0009's `transport_time` and `host_time`
/// domains: it is never meaningful to compare two `MonoTimestamp` values that
/// originated on different endpoints without first correlating clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonoTimestamp(u64);

impl MonoTimestamp {
    /// Constructs a timestamp from a raw nanosecond count.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// Returns the timestamp as a raw nanosecond count.
    #[must_use]
    pub const fn as_nanos(&self) -> u64 {
        self.0
    }

    /// Adds a duration, saturating at `u64::MAX` nanoseconds instead of
    /// overflowing.
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        let added = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        Self(self.0.saturating_add(added))
    }

    /// Returns the elapsed duration from `earlier` to `self`, saturating at
    /// zero if `earlier` is later than `self`.
    #[must_use]
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

/// A simulation tick counter, distinct from wall-clock or transport time.
///
/// Corresponds to ADR-0009's `simulation_time` domain: an adapter's simulated
/// clock, which may run slower, faster, or stepped relative to wall time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SimTick(u64);

impl SimTick {
    /// Constructs a tick from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the tick as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the next tick, wrapping on overflow rather than panicking.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Duration, MonoTimestamp, SimTick};

    #[test]
    fn mono_timestamp_roundtrips_nanos() {
        let ts = MonoTimestamp::from_nanos(42);
        assert_eq!(ts.as_nanos(), 42);
    }

    #[test]
    fn mono_timestamp_saturating_add_saturates() {
        let ts = MonoTimestamp::from_nanos(u64::MAX - 5);
        let added = ts.saturating_add(Duration::from_nanos(100));
        assert_eq!(added.as_nanos(), u64::MAX);
    }

    #[test]
    fn mono_timestamp_duration_since_saturates_at_zero() {
        let earlier = MonoTimestamp::from_nanos(100);
        let later = MonoTimestamp::from_nanos(50);
        assert_eq!(
            later.saturating_duration_since(earlier),
            Duration::from_nanos(0)
        );
    }

    #[test]
    fn mono_timestamp_duration_since_computes_delta() {
        let earlier = MonoTimestamp::from_nanos(50);
        let later = MonoTimestamp::from_nanos(150);
        assert_eq!(
            later.saturating_duration_since(earlier),
            Duration::from_nanos(100)
        );
    }

    #[test]
    fn sim_tick_next_advances() {
        let tick = SimTick::new(0);
        assert_eq!(tick.next().as_u64(), 1);
    }

    #[test]
    fn sim_tick_next_wraps_on_overflow() {
        let tick = SimTick::new(u64::MAX);
        assert_eq!(tick.next().as_u64(), 0);
    }
}
