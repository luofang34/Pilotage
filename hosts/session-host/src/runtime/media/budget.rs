//! Per-connection aggregate video admission and transport-pressure feedback.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// The pre-transport aggregate ceiling for one client's video.
pub(super) const MAX_BYTES_PER_SECOND: u64 = 2_000_000;
const MIN_BYTES_PER_SECOND: u64 = 125_000;
const RECOVERY_STEP_BYTES_PER_SECOND: u64 = 250_000;
const FEEDBACK_INTERVAL_NS: u64 = 250_000_000;
const QUIET_RECOVERY_NS: u64 = 5_000_000_000;
const BURST_DIVISOR: u64 = 4;
const MIN_BURST_BYTES: u64 = 256_000;
const SOURCE_ACTIVE_NS: u64 = 1_000_000_000;

/// A viewer-visible video admission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeliveryMode {
    Normal,
    Degraded,
    Suspended,
}

/// The current mode and aggregate byte rate for one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeliveryState {
    pub(super) mode: DeliveryMode,
    pub(super) bytes_per_second: u64,
}

/// Cumulative QUIC path counters sampled without resetting transport state.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TransportSnapshot {
    pub(super) lost_packets: u64,
    pub(super) congestion_events: u64,
}

/// Cross-writer pressure counters shared by every source of one client.
#[derive(Debug, Default)]
pub(super) struct PressureSignals {
    busy_drops: AtomicU64,
    deadline_stalls: AtomicU64,
    active_reapers: AtomicUsize,
}

impl PressureSignals {
    pub(super) fn record_busy_drop(&self) {
        self.busy_drops
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.wrapping_add(1))
            })
            .ok();
    }

    pub(super) fn record_deadline_stall(&self) {
        self.deadline_stalls
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.wrapping_add(1))
            })
            .ok();
    }

    pub(super) fn reaper_started(&self) {
        self.active_reapers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_add(1))
            })
            .ok();
    }

    pub(super) fn reaper_finished(&self) {
        self.active_reapers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_sub(1))
            })
            .ok();
    }

    pub(super) fn snapshot(&self) -> PressureSnapshot {
        PressureSnapshot {
            busy_drops: self.busy_drops.load(Ordering::Acquire),
            deadline_stalls: self.deadline_stalls.load(Ordering::Acquire),
            active_reapers: self.active_reapers.load(Ordering::Acquire),
        }
    }
}

/// One coherent sample of media-writer pressure.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PressureSnapshot {
    busy_drops: u64,
    deadline_stalls: u64,
    pub(super) active_reapers: usize,
}

/// Result of considering one encoded frame for a connection.
pub(super) struct Admission {
    pub(super) admitted: bool,
    pub(super) transition: Option<DeliveryState>,
}

/// Token bucket plus AIMD controller for one connection's aggregate video.
pub(super) struct SendBudget {
    rate: u64,
    tokens: u64,
    last_refill_ns: u64,
    last_feedback_ns: u64,
    last_pressure_ns: u64,
    last_rate_change_ns: u64,
    last_budget_drop_ns: Option<u64>,
    pressure_baseline: PressureSnapshot,
    transport_baseline: TransportSnapshot,
    published: DeliveryState,
    sources: BTreeMap<u8, SourceBucket>,
}

struct SourceBucket {
    tokens: u64,
    last_seen_ns: u64,
}

impl SendBudget {
    pub(super) fn new(now_ns: u64, transport: TransportSnapshot) -> Self {
        Self {
            rate: MAX_BYTES_PER_SECOND,
            tokens: bucket_capacity(MAX_BYTES_PER_SECOND),
            last_refill_ns: now_ns,
            last_feedback_ns: now_ns,
            last_pressure_ns: now_ns,
            last_rate_change_ns: now_ns,
            last_budget_drop_ns: None,
            pressure_baseline: PressureSnapshot::default(),
            transport_baseline: transport,
            published: DeliveryState {
                mode: DeliveryMode::Normal,
                bytes_per_second: MAX_BYTES_PER_SECOND,
            },
            sources: BTreeMap::new(),
        }
    }

    pub(super) fn admit(
        &mut self,
        now_ns: u64,
        source_id: u8,
        bytes: usize,
        pressure: PressureSnapshot,
        transport: TransportSnapshot,
    ) -> Admission {
        self.evaluate_feedback(now_ns, pressure, transport);
        self.refill(now_ns);
        let capacity = source_capacity(self.rate, self.sources.len().saturating_add(1));
        let source = self.sources.entry(source_id).or_insert(SourceBucket {
            tokens: capacity,
            last_seen_ns: now_ns,
        });
        source.last_seen_ns = now_ns;
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let admitted = self.rate > 0 && bytes <= self.tokens && bytes <= source.tokens;
        if admitted {
            self.tokens = self.tokens.saturating_sub(bytes);
            source.tokens = source.tokens.saturating_sub(bytes);
        } else {
            self.last_budget_drop_ns = Some(now_ns);
        }
        Admission {
            admitted,
            transition: self.take_transition(now_ns),
        }
    }

    pub(super) fn feedback_due(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_feedback_ns) >= FEEDBACK_INTERVAL_NS
    }

    fn evaluate_feedback(
        &mut self,
        now_ns: u64,
        pressure: PressureSnapshot,
        transport: TransportSnapshot,
    ) {
        if now_ns.saturating_sub(self.last_feedback_ns) < FEEDBACK_INTERVAL_NS {
            return;
        }
        self.last_feedback_ns = now_ns;
        let pressured = pressure
            .busy_drops
            .wrapping_sub(self.pressure_baseline.busy_drops)
            > 0
            || pressure
                .deadline_stalls
                .wrapping_sub(self.pressure_baseline.deadline_stalls)
                > 0
            || pressure.active_reapers > 0
            || transport
                .lost_packets
                .wrapping_sub(self.transport_baseline.lost_packets)
                > 0
            || transport
                .congestion_events
                .wrapping_sub(self.transport_baseline.congestion_events)
                > 0;
        self.pressure_baseline = pressure;
        self.transport_baseline = transport;
        if pressured {
            self.last_pressure_ns = now_ns;
            self.decrease(now_ns);
        } else if now_ns.saturating_sub(self.last_pressure_ns) >= QUIET_RECOVERY_NS
            && now_ns.saturating_sub(self.last_rate_change_ns) >= QUIET_RECOVERY_NS
        {
            self.increase(now_ns);
        }
    }

    fn decrease(&mut self, now_ns: u64) {
        self.rate = if self.rate <= MIN_BYTES_PER_SECOND {
            0
        } else {
            (self.rate / 2).max(MIN_BYTES_PER_SECOND)
        };
        self.tokens = self.tokens.min(bucket_capacity(self.rate));
        self.clamp_source_tokens();
        self.last_rate_change_ns = now_ns;
    }

    fn increase(&mut self, now_ns: u64) {
        self.rate = if self.rate == 0 {
            MIN_BYTES_PER_SECOND
        } else {
            self.rate
                .saturating_add(RECOVERY_STEP_BYTES_PER_SECOND)
                .min(MAX_BYTES_PER_SECOND)
        };
        self.tokens = self.tokens.min(bucket_capacity(self.rate));
        self.clamp_source_tokens();
        self.last_rate_change_ns = now_ns;
        self.last_pressure_ns = now_ns;
    }

    fn refill(&mut self, now_ns: u64) {
        let elapsed_ns = now_ns.saturating_sub(self.last_refill_ns);
        let refill =
            u128::from(self.rate).saturating_mul(u128::from(elapsed_ns)) / 1_000_000_000_u128;
        let refill = u64::try_from(refill).unwrap_or(u64::MAX);
        self.tokens = self
            .tokens
            .saturating_add(refill)
            .min(bucket_capacity(self.rate));
        self.sources
            .retain(|_, source| now_ns.saturating_sub(source.last_seen_ns) <= SOURCE_ACTIVE_NS);
        let active = self.sources.len().max(1);
        let source_refill = refill / u64::try_from(active).unwrap_or(u64::MAX);
        let capacity = source_capacity(self.rate, active);
        for source in self.sources.values_mut() {
            source.tokens = source.tokens.saturating_add(source_refill).min(capacity);
        }
        self.last_refill_ns = now_ns;
    }

    fn clamp_source_tokens(&mut self) {
        let capacity = source_capacity(self.rate, self.sources.len().max(1));
        for source in self.sources.values_mut() {
            source.tokens = source.tokens.min(capacity);
        }
    }

    fn take_transition(&mut self, now_ns: u64) -> Option<DeliveryState> {
        let limited = self
            .last_budget_drop_ns
            .is_some_and(|last| now_ns.saturating_sub(last) < QUIET_RECOVERY_NS);
        let state = DeliveryState {
            mode: if self.rate == 0 {
                DeliveryMode::Suspended
            } else if self.rate < MAX_BYTES_PER_SECOND || limited {
                DeliveryMode::Degraded
            } else {
                DeliveryMode::Normal
            },
            bytes_per_second: self.rate,
        };
        if state == self.published {
            None
        } else {
            self.published = state;
            Some(state)
        }
    }
}

fn bucket_capacity(rate: u64) -> u64 {
    if rate == 0 {
        0
    } else {
        (rate / BURST_DIVISOR).max(MIN_BURST_BYTES)
    }
}

fn source_capacity(rate: u64, active: usize) -> u64 {
    if rate == 0 {
        return 0;
    }
    let active = u64::try_from(active).unwrap_or(u64::MAX).max(1);
    (rate / BURST_DIVISOR / active).max(MIN_BURST_BYTES)
}

#[cfg(test)]
mod tests;
