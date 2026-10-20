//! Pose-to-frame latency as the client sees it.
//!
//! A world-model camera renders each frame from one simulator-truth sample
//! and stamps the frame with that sample's acquisition time (ADR-0020). The
//! same sample reaches this client in telemetry. So the gap between the
//! client receiving a truth sample and the client receiving a frame captured
//! at that sample's time is the camera's whole pose-to-frame latency,
//! measured on one clock: rendering, readback, encoding, and both transports.

use std::collections::BTreeMap;
use std::time::Duration;

use pilotage_timing::MonoTimestamp;

use crate::metrics::Histogram;

/// Truth samples remembered for matching; a frame older than this many
/// samples finds none and is not counted.
const TRUTH_WINDOW: usize = 4096;
/// Latency samples kept per camera.
const HISTOGRAM_CAPACITY: usize = 10_000;
/// Video sources measured: FPV (0), chase (1), gimbal (2).
const SOURCES: usize = 3;

/// Matches frames to the truth samples they were rendered from.
pub struct PoseToFrame {
    /// Client receive time of each truth sample, by (clock code, acquisition
    /// time).
    truth: BTreeMap<(u8, u64), MonoTimestamp>,
    /// Latency per video source.
    latency: [Histogram; SOURCES],
}

impl Default for PoseToFrame {
    fn default() -> Self {
        Self {
            truth: BTreeMap::new(),
            latency: std::array::from_fn(|_| Histogram::new(HISTOGRAM_CAPACITY)),
        }
    }
}

impl PoseToFrame {
    /// Notes when a truth sample acquired at `acquired_at_ns` on wire clock
    /// `clock` reached this client. A sample re-broadcast unchanged keeps its
    /// first receipt.
    pub fn note_truth(&mut self, clock: i32, acquired_at_ns: u64, received_at: MonoTimestamp) {
        let Ok(clock) = u8::try_from(clock) else {
            return;
        };
        self.truth
            .entry((clock, acquired_at_ns))
            .or_insert(received_at);
        while self.truth.len() > TRUTH_WINDOW {
            self.truth.pop_first();
        }
    }

    /// Records a frame of `source_id` captured at `capture_time_ns` on clock
    /// code `capture_clock`, received at `received_at`. A frame no noted
    /// truth sample matches, or that arrived before it, is not counted.
    pub fn note_frame(
        &mut self,
        source_id: u8,
        capture_clock: u8,
        capture_time_ns: u64,
        received_at: MonoTimestamp,
    ) {
        let Some(histogram) = self.latency.get_mut(usize::from(source_id)) else {
            return;
        };
        let Some(truth_at) = self.truth.get(&(capture_clock, capture_time_ns)) else {
            return;
        };
        if let Some(gap) = received_at.as_nanos().checked_sub(truth_at.as_nanos()) {
            histogram.record(Duration::from_nanos(gap));
        }
    }

    /// (p50, p95, max, count) of `source_id`'s latency, or `None` with no
    /// matched frame.
    #[must_use]
    pub fn summary(&self, source_id: u8) -> Option<(Duration, Duration, Duration, usize)> {
        let histogram = self.latency.get(usize::from(source_id))?;
        let (p50, p95, max) = histogram.percentiles()?;
        Some((p50, p95, max, histogram.len()))
    }
}

#[cfg(test)]
mod tests;
