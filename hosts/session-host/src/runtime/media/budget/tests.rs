#![allow(clippy::expect_used, clippy::panic)]

use super::*;

const FRAME_BYTES: usize = 30_000;
const FRAME_INTERVAL_NS: u64 = 1_000_000_000 / 90;

#[test]
fn three_full_rate_sources_share_a_bounded_aggregate() {
    let mut budget = SendBudget::new(0, TransportSnapshot::default());
    let pressure = PressureSnapshot::default();
    let mut admitted = [0_u64; 3];
    let duration_ns = 10_000_000_000_u64;
    let mut now_ns = 0_u64;
    let mut frame_index = 0_usize;
    while now_ns < duration_ns {
        let source = frame_index % admitted.len();
        if budget
            .admit(
                now_ns,
                u8::try_from(source).expect("three source ids fit u8"),
                FRAME_BYTES,
                pressure,
                TransportSnapshot::default(),
            )
            .admitted
        {
            admitted[source] = admitted[source].wrapping_add(FRAME_BYTES as u64);
        }
        frame_index = frame_index.wrapping_add(1);
        now_ns = now_ns.saturating_add(FRAME_INTERVAL_NS);
    }
    let total: u64 = admitted.iter().sum();
    let allowed = MAX_BYTES_PER_SECOND
        .saturating_mul(duration_ns / 1_000_000_000)
        .saturating_add(bucket_capacity(MAX_BYTES_PER_SECOND));
    assert!(total <= allowed, "token bucket bounds aggregate bytes");
    assert!(
        admitted.iter().all(|bytes| *bytes > 0),
        "all sources retain service"
    );
    let least = admitted.iter().min().expect("three sources");
    let most = admitted.iter().max().expect("three sources");
    assert!(
        most.saturating_mul(100) <= least.saturating_mul(105),
        "three active sources stay within five percent of one another: {admitted:?}"
    );
    assert_eq!(budget.published.mode, DeliveryMode::Degraded);
}

#[test]
fn writer_pressure_can_suspend_then_quietly_probe_upward() {
    let signals = PressureSignals::default();
    let mut budget = SendBudget::new(0, TransportSnapshot::default());
    let mut now_ns = FEEDBACK_INTERVAL_NS;
    while budget.rate > 0 {
        signals.record_busy_drop();
        budget.admit(
            now_ns,
            0,
            1,
            signals.snapshot(),
            TransportSnapshot::default(),
        );
        now_ns = now_ns.saturating_add(FEEDBACK_INTERVAL_NS);
    }
    assert_eq!(budget.published.mode, DeliveryMode::Suspended);

    now_ns = now_ns.saturating_add(QUIET_RECOVERY_NS);
    budget.admit(
        now_ns,
        0,
        1,
        signals.snapshot(),
        TransportSnapshot::default(),
    );
    assert_eq!(budget.rate, MIN_BYTES_PER_SECOND);
    assert_eq!(budget.published.mode, DeliveryMode::Degraded);
}

#[test]
fn quic_loss_deadlines_and_live_reapers_are_adaptive_pressure() {
    let signals = PressureSignals::default();
    let mut budget = SendBudget::new(0, TransportSnapshot::default());
    let loss = TransportSnapshot {
        lost_packets: 1,
        congestion_events: 1,
    };
    budget.admit(FEEDBACK_INTERVAL_NS, 0, 1, signals.snapshot(), loss);
    assert_eq!(budget.rate, MAX_BYTES_PER_SECOND / 2);

    signals.record_deadline_stall();
    budget.admit(FEEDBACK_INTERVAL_NS * 2, 0, 1, signals.snapshot(), loss);
    assert_eq!(budget.rate, MAX_BYTES_PER_SECOND / 4);

    signals.reaper_started();
    budget.admit(FEEDBACK_INTERVAL_NS * 3, 0, 1, signals.snapshot(), loss);
    assert_eq!(budget.rate, MAX_BYTES_PER_SECOND / 8);
    signals.reaper_finished();
    assert_eq!(signals.snapshot().active_reapers, 0);
}
