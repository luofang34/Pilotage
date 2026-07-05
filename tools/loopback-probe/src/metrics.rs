//! Run-level measurement accumulation: control-frame round-trip-to-telemetry
//! latency, Ping/Pong RTT, and frame accept/reject/telemetry counters.
//!
//! Latency here is measured entirely on the client's own clock (loopback,
//! same machine): a control frame's client send timestamp is compared
//! against the client-observed receive timestamp of the first telemetry
//! sample whose pose changed after it, both read from the same monotonic
//! clock. This sidesteps ADR-0009's cross-endpoint comparison ban because
//! no host-side timestamp is used in the computation at all; the tradeoff
//! documented for readers is that this measures software + loopback-network
//! latency only, not true cross-host wire time (ADR-0009's "local-loopback
//! mode is the baseline that separates software latency from network
//! latency").

use std::time::Duration;

use pilotage_timing::RttEstimator;

/// A fixed-capacity, sorted-on-read latency histogram. Bounded so a long
/// run cannot grow this without limit; the oldest sample is evicted and
/// counted as dropped once full (mirrors `pilotage_timing::BoundedLatencyLog`'s
/// eviction policy, but over a plain `Duration` since these samples are not
/// tied to a `Stage`).
pub struct Histogram {
    samples: Vec<Duration>,
    capacity: usize,
    dropped: u64,
}

impl Histogram {
    /// Constructs an empty histogram bounded to `capacity` retained samples.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::new(),
            capacity,
            dropped: 0,
        }
    }

    /// Records one latency sample, evicting the oldest once at capacity.
    pub fn record(&mut self, sample: Duration) {
        if self.samples.len() >= self.capacity {
            self.samples.remove(0);
            self.dropped = self.dropped.wrapping_add(1);
        }
        self.samples.push(sample);
    }

    /// Number of samples currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Number of samples evicted by capacity overrun.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Returns (p50, p95, max), or `None` if no samples were recorded.
    #[must_use]
    pub fn percentiles(&self) -> Option<(Duration, Duration, Duration)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let p50 = percentile(&sorted, 50);
        let p95 = percentile(&sorted, 95);
        #[allow(clippy::expect_used)]
        let max = *sorted.last().expect("checked non-empty above");
        Some((p50, p95, max))
    }
}

/// Nearest-rank percentile over an already-sorted, non-empty slice.
fn percentile(sorted: &[Duration], pct: usize) -> Duration {
    let len = sorted.len();
    let rank = (pct * len).div_ceil(100).clamp(1, len);
    sorted[rank - 1]
}

/// Accumulates every measurement this probe reports at the end of a run.
pub struct RunMetrics {
    /// Control-send -> matching-telemetry-change latency.
    pub control_to_telemetry: Histogram,
    /// Control frames evicted from the pending-match queue without ever
    /// producing a latency sample. Two paths feed this: a frame sent at or
    /// before the last telemetry sample's receive time (already reflected in
    /// that pose, so a later change cannot causally belong to it), and a frame
    /// dropped from the front because the queue reached its capacity bound
    /// while telemetry lagged. Distinct from `Histogram::dropped`, which counts
    /// samples that *were* matched but then evicted for exceeding the
    /// histogram's own retention capacity.
    pub control_to_telemetry_backlog_dropped: u64,
    /// Ping/Pong round-trip time estimator.
    pub rtt: RttEstimator,
    /// Control frames sent (datagrams written).
    pub frames_sent: u64,
    /// `FrameRejected` notices received from the host.
    pub frames_rejected: u64,
    /// Telemetry samples received.
    pub telemetry_received: u64,
}

impl RunMetrics {
    /// Constructs an empty metrics accumulator with `histogram_capacity`
    /// retained latency samples.
    #[must_use]
    pub const fn new(histogram_capacity: usize) -> Self {
        Self {
            control_to_telemetry: Histogram::new(histogram_capacity),
            control_to_telemetry_backlog_dropped: 0,
            rtt: RttEstimator::new(),
            frames_sent: 0,
            frames_rejected: 0,
            telemetry_received: 0,
        }
    }

    /// Frames accepted: sent minus explicitly rejected. Not a direct host
    /// signal (the host does not ack accepts) — inferred as the complement
    /// of rejection, which is exact as long as every sent frame either
    /// lands or is rejected (true for the loopback path this tool targets).
    #[must_use]
    pub fn frames_accepted(&self) -> u64 {
        self.frames_sent.saturating_sub(self.frames_rejected)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::Histogram;
    use std::time::Duration;

    #[test]
    fn percentiles_of_single_sample() {
        let mut hist = Histogram::new(10);
        hist.record(Duration::from_millis(5));
        let (p50, p95, max) = hist.percentiles().expect("has samples");
        assert_eq!(p50, Duration::from_millis(5));
        assert_eq!(p95, Duration::from_millis(5));
        assert_eq!(max, Duration::from_millis(5));
    }

    #[test]
    fn percentiles_over_sorted_range() {
        let mut hist = Histogram::new(100);
        for ms in 1..=100u64 {
            hist.record(Duration::from_millis(ms));
        }
        let (p50, p95, max) = hist.percentiles().expect("has samples");
        assert_eq!(p50, Duration::from_millis(50));
        assert_eq!(p95, Duration::from_millis(95));
        assert_eq!(max, Duration::from_millis(100));
    }

    #[test]
    fn empty_histogram_has_no_percentiles() {
        let hist = Histogram::new(10);
        assert!(hist.percentiles().is_none());
    }

    #[test]
    fn overflow_evicts_oldest_and_counts_drop() {
        let mut hist = Histogram::new(2);
        hist.record(Duration::from_millis(1));
        hist.record(Duration::from_millis(2));
        hist.record(Duration::from_millis(3));
        assert_eq!(hist.len(), 2);
        assert_eq!(hist.dropped(), 1);
    }
}
