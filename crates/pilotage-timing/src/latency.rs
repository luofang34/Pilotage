//! Per-stage latency records and a bounded ring buffer for accumulating them.
//!
//! ADR-0009 requires end-to-end latency accounting across every named stage
//! of the control and video loops. `BoundedLatencyLog` gives hosts a fixed-
//! capacity store so latency observability cannot cause unbounded memory
//! growth, and treats buffer overrun (dropped samples) as a first-class,
//! counted signal rather than silent data loss.

use core::time::Duration;

/// A named stage in the end-to-end control or video loop (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Device input sampling.
    Sample,
    /// Serializing a control frame for transmission.
    Serialize,
    /// Network transit from client to host.
    NetworkUplink,
    /// Host-side validation of a received frame.
    Validate,
    /// Applying validated input to the simulation.
    Apply,
    /// Rendering a frame for output.
    Render,
    /// Encoding a rendered frame for transmission.
    Encode,
    /// Network transit from host to client.
    NetworkDownlink,
    /// Decoding a received video frame.
    Decode,
    /// Presenting a decoded frame to the user.
    Present,
}

/// A single measured duration for one `Stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageLatency {
    /// Which stage this measurement belongs to.
    pub stage: Stage,
    /// How long the stage took.
    pub duration: Duration,
}

impl StageLatency {
    /// Constructs a new stage-latency record.
    #[must_use]
    pub const fn new(stage: Stage, duration: Duration) -> Self {
        Self { stage, duration }
    }
}

/// A fixed-capacity ring buffer of `StageLatency` records.
///
/// `CAPACITY` is a const generic so the buffer never allocates and its memory
/// footprint is known at compile time. When full, `push` overwrites the
/// oldest entry and increments a wrapping drop counter: drops are a
/// correctness signal (a stage falling behind observability), not merely a
/// storage detail, so the count is always accessible via `dropped`.
#[derive(Debug, Clone)]
pub struct BoundedLatencyLog<const CAPACITY: usize> {
    entries: [Option<StageLatency>; CAPACITY],
    next_write: usize,
    len: usize,
    dropped: u64,
}

impl<const CAPACITY: usize> BoundedLatencyLog<CAPACITY> {
    /// Constructs an empty log. `CAPACITY` must be nonzero for `push` to ever
    /// retain an entry; a zero-capacity log is legal but drops everything.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            next_write: 0,
            len: 0,
            dropped: 0,
        }
    }

    /// Pushes a new record, overwriting the oldest entry once full.
    ///
    /// Returns `true` if the record was stored without overwriting an
    /// existing entry, `false` if an older entry was evicted (counted in
    /// `dropped`).
    pub fn push(&mut self, record: StageLatency) -> bool {
        if CAPACITY == 0 {
            self.dropped = self.dropped.wrapping_add(1);
            return false;
        }
        let overwriting = self.entries[self.next_write].is_some();
        self.entries[self.next_write] = Some(record);
        self.next_write = (self.next_write + 1) % CAPACITY;
        if overwriting {
            self.dropped = self.dropped.wrapping_add(1);
        } else {
            self.len += 1;
        }
        !overwriting
    }

    /// Returns the number of records currently stored (not the drop count).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if no records are stored.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the count of records evicted by overwrite, via `wrapping_add`
    /// so a long-running host never panics on counter overflow.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Iterates over currently-stored records in oldest-to-newest order.
    pub fn iter(&self) -> impl Iterator<Item = &StageLatency> {
        let start = if self.len == CAPACITY {
            self.next_write
        } else {
            0
        };
        (0..self.len).filter_map(move |offset| {
            let index = (start + offset) % CAPACITY.max(1);
            self.entries.get(index).and_then(Option::as_ref)
        })
    }

    /// Returns the record with the maximum duration, if any are stored.
    #[must_use]
    pub fn max(&self) -> Option<StageLatency> {
        self.iter().copied().max_by_key(|record| record.duration)
    }

    /// Returns the mean duration across all stored records, if any.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        if self.len == 0 {
            return None;
        }
        let total_nanos: u128 = self.iter().map(|record| record.duration.as_nanos()).sum();
        let len = u128::try_from(self.len).unwrap_or(1);
        let mean_nanos = total_nanos / len;
        Some(Duration::from_nanos(
            u64::try_from(mean_nanos).unwrap_or(u64::MAX),
        ))
    }
}

impl<const CAPACITY: usize> Default for BoundedLatencyLog<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{BoundedLatencyLog, Duration, Stage, StageLatency};

    #[test]
    fn push_below_capacity_does_not_drop() {
        let mut log: BoundedLatencyLog<4> = BoundedLatencyLog::new();
        assert!(log.push(StageLatency::new(Stage::Sample, Duration::from_millis(1))));
        assert_eq!(log.len(), 1);
        assert_eq!(log.dropped(), 0);
    }

    #[test]
    fn push_past_capacity_wraps_and_counts_drops() {
        let mut log: BoundedLatencyLog<2> = BoundedLatencyLog::new();
        log.push(StageLatency::new(Stage::Sample, Duration::from_millis(1)));
        log.push(StageLatency::new(
            Stage::Serialize,
            Duration::from_millis(2),
        ));
        assert_eq!(log.dropped(), 0);
        let stored = log.push(StageLatency::new(Stage::Validate, Duration::from_millis(3)));
        assert!(!stored);
        assert_eq!(log.dropped(), 1);
        assert_eq!(log.len(), 2);
        let durations: Vec<Duration> = log.iter().map(|r| r.duration).collect();
        assert_eq!(
            durations,
            vec![Duration::from_millis(2), Duration::from_millis(3)]
        );
    }

    #[test]
    fn max_and_mean_over_records() {
        let mut log: BoundedLatencyLog<3> = BoundedLatencyLog::new();
        log.push(StageLatency::new(Stage::Sample, Duration::from_millis(10)));
        log.push(StageLatency::new(Stage::Render, Duration::from_millis(30)));
        log.push(StageLatency::new(Stage::Encode, Duration::from_millis(20)));
        assert_eq!(
            log.max().expect("max present").duration,
            Duration::from_millis(30)
        );
        assert_eq!(log.mean(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn empty_log_has_no_max_or_mean() {
        let log: BoundedLatencyLog<3> = BoundedLatencyLog::new();
        assert!(log.is_empty());
        assert_eq!(log.max(), None);
        assert_eq!(log.mean(), None);
    }

    #[test]
    fn dropped_counter_wraps_on_overflow() {
        let mut log: BoundedLatencyLog<1> = BoundedLatencyLog::new();
        log.dropped = u64::MAX;
        log.push(StageLatency::new(Stage::Sample, Duration::from_millis(1)));
        let dropped = log.push(StageLatency::new(Stage::Render, Duration::from_millis(1)));
        assert!(!dropped);
        assert_eq!(log.dropped(), 0);
    }
}
