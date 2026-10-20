use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{MeasurementClock, SourceIncarnation, SourceIntegrity, SourceRole};
use pilotage_protocol::LandedState;

use super::{ArmTracker, fc_state_sample, landed_state_sample};
use crate::codec::{
    EXTENDED_SYS_STATE_ID, GLOBAL_POSITION_INT_ID, HEARTBEAT_ID, encode_frame_v2_from,
};
use crate::link::{ResetPolicy, apply_messages_at};
use crate::sampling::WITHHOLD_AFTER;
use crate::{LinkState, parse_datagram};

fn state(start: Instant) -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState {
        system_id: 23,
        component_id: 1,
        source_id: 42,
        source_incarnation: SourceIncarnation::new([7; 16]),
        started_at: start,
        reset_policy: ResetPolicy::SimulatorHeuristic,
        ..LinkState::default()
    }))
}

fn receive(
    state: &Arc<Mutex<LinkState>>,
    id: u32,
    payload: &[u8],
    crc: u8,
    at: Instant,
    system: u8,
) {
    let mut bytes = vec![0; payload.len() + 12];
    let count = encode_frame_v2_from(0, id, payload, crc, system, 1, &mut bytes);
    let mut messages = Vec::new();
    let stats = parse_datagram(&bytes[..count], &mut messages);
    assert_eq!(stats.decoded, 1);
    apply_messages_at(state, &messages, stats.crc_failures, stats.unknown_ids, at);
}

fn heartbeat(state: &Arc<Mutex<LinkState>>, armed: bool, at: Instant) {
    let mut payload = [0; 9];
    payload[..4].copy_from_slice(&4_u32.to_le_bytes());
    payload[4] = 2;
    payload[5] = 3;
    payload[6] = if armed { 0x80 } else { 0 };
    payload[7] = 4;
    payload[8] = 3;
    receive(state, HEARTBEAT_ID, &payload, 50, at, 23);
}

fn altitude(state: &Arc<Mutex<LinkState>>, time_boot_ms: u32, at: Instant) {
    let mut payload = [0; 28];
    payload[..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    receive(state, GLOBAL_POSITION_INT_ID, &payload, 104, at, 23);
}

fn landed(state: &Arc<Mutex<LinkState>>, value: u8, at: Instant) {
    receive(state, EXTENDED_SYS_STATE_ID, &[0, value], 130, at, 23);
}

#[test]
fn heartbeat_receive_time_and_landed_receive_time_remain_independent() {
    let start = Instant::now();
    let state = state(start);
    let mut tracker = ArmTracker::default();
    tracker.observe(&state.lock().expect("cache"));
    assert!(tracker.report().is_none());

    let arm_at = start + Duration::from_millis(100);
    let landed_at = start + Duration::from_millis(160);
    heartbeat(&state, false, arm_at);
    landed(&state, 1, landed_at);
    let latest = state.lock().expect("cache");
    tracker.observe(&latest);
    let report = tracker.report().expect("heartbeat report");
    let landed = landed_state_sample(&latest, landed_at).expect("landed report");
    let sample = fc_state_sample(Some(report), None, Some(landed)).expect("FC sample");
    assert_eq!(sample.arm_state, 1);
    let stamp = sample.stamp.expect("arm stamp");
    assert_eq!(stamp, report.stamp);
    assert_eq!(stamp.source_id, 42);
    assert_eq!(stamp.source_incarnation, SourceIncarnation::new([7; 16]));
    assert_eq!(stamp.source_epoch, latest.source_epoch);
    assert_eq!(stamp.role, SourceRole::FcState);
    assert_eq!(stamp.integrity, SourceIntegrity::ChecksummedOnly);
    assert_eq!(stamp.clock, MeasurementClock::HostMonotonic);
    assert_eq!(stamp.acquired_at_ns, 100_000_000);
    assert_eq!(landed.stamp.acquired_at_ns, 160_000_000);
    assert_eq!(landed.stamp.source_incarnation, stamp.source_incarnation);
    assert_eq!(landed.stamp.source_epoch, stamp.source_epoch);
    assert_eq!(landed.stamp.source_id, stamp.source_id);
    assert_eq!(sample.landed_state, Some(landed));
}

#[test]
fn resampling_does_not_refresh_an_arm_or_landed_observation() {
    let start = Instant::now();
    let state = state(start);
    heartbeat(&state, false, start);
    landed(&state, 1, start);
    let mut tracker = ArmTracker::default();
    let first = {
        let latest = state.lock().expect("cache");
        tracker.observe(&latest);
        tracker.report().expect("first heartbeat")
    };
    {
        let latest = state.lock().expect("cache");
        tracker.observe(&latest);
        assert_eq!(tracker.report(), Some(first));
        let fresh = landed_state_sample(&latest, start).expect("fresh landed state");
        let repeated = landed_state_sample(&latest, start + WITHHOLD_AFTER);
        assert_eq!(repeated, Some(fresh));
        assert!(
            landed_state_sample(&latest, start + WITHHOLD_AFTER + Duration::from_nanos(1))
                .is_none()
        );
        assert!(landed_state_sample(&latest, start - Duration::from_nanos(1)).is_none());
    }
    heartbeat(&state, true, start + Duration::from_secs(1));
    tracker.observe(&state.lock().expect("cache"));
    let second = tracker.report().expect("second heartbeat");
    assert!(second.armed);
    assert_eq!(second.stamp.sequence, first.stamp.sequence.wrapping_add(1));
    assert_eq!(second.stamp.acquired_at_ns, 1_000_000_000);
}

#[test]
fn all_landed_values_preserve_the_decoded_report_stamp() {
    let start = Instant::now();
    let state = state(start);
    for (value, expected) in [
        (0, LandedState::Unknown),
        (1, LandedState::OnGround),
        (2, LandedState::InAir),
        (3, LandedState::TakingOff),
        (4, LandedState::Landing),
        (255, LandedState::Unknown),
    ] {
        let at = start + Duration::from_millis(u64::from(value));
        landed(&state, value, at);
        let latest = state.lock().expect("cache");
        let report = latest
            .fc_reports
            .landed_state
            .expect("decoded landed report");
        let sample = landed_state_sample(&latest, at).expect("current landed report");
        assert_eq!(sample.state, expected);
        assert_eq!(sample.stamp, report.stamp);
    }
}

#[test]
fn a_confirmed_source_reset_clears_arm_until_a_new_epoch_heartbeat_arrives() {
    let start = Instant::now();
    let state = state(start);
    let mut tracker = ArmTracker::default();
    altitude(&state, 10_000, start);
    heartbeat(&state, true, start);
    landed(&state, 2, start);
    tracker.observe(&state.lock().expect("cache"));
    let first = tracker.report().expect("initial heartbeat");
    let candidate = start + Duration::from_secs(3);
    altitude(&state, 100, candidate);
    {
        let latest = state.lock().expect("cache");
        assert!(latest.pending_reset.is_some());
        tracker.observe(&latest);
        assert_eq!(tracker.report(), Some(first));
        assert!(landed_state_sample(&latest, candidate).is_none());
    }
    let confirmed = candidate + Duration::from_millis(250);
    altitude(&state, 350, confirmed);
    {
        let latest = state.lock().expect("cache");
        assert_eq!(latest.source_resets, 1);
        assert!(latest.last_heartbeat.is_none());
        tracker.observe(&latest);
        assert!(tracker.report().is_none());
        assert!(fc_state_sample(tracker.report(), None, None).is_none());
        assert!(landed_state_sample(&latest, confirmed).is_none());
    }
    let received = confirmed + Duration::from_millis(1);
    heartbeat(&state, false, received);
    landed(&state, 1, received);
    let latest = state.lock().expect("cache");
    tracker.observe(&latest);
    let report = tracker.report().expect("new source epoch heartbeat");
    let landed = landed_state_sample(&latest, received).expect("new source epoch landed");
    assert!(!report.armed);
    assert_eq!(
        report.stamp.source_epoch,
        first.stamp.source_epoch.wrapping_add(1)
    );
    assert_eq!(report.stamp.sequence, 0);
    assert_eq!(report.stamp.acquired_at_ns, 3_251_000_000);
    assert_eq!(report.stamp.source_epoch, landed.stamp.source_epoch);
    assert_eq!(
        report.stamp.source_incarnation,
        landed.stamp.source_incarnation
    );
}

#[test]
fn an_unselected_source_cannot_supply_arm_or_landed_observations() {
    let start = Instant::now();
    let state = state(start);
    receive(
        &state,
        HEARTBEAT_ID,
        &[0, 0, 0, 0, 2, 3, 128, 4, 3],
        50,
        start,
        24,
    );
    receive(&state, EXTENDED_SYS_STATE_ID, &[0, 1], 130, start, 24);
    let latest = state.lock().expect("cache");
    let mut tracker = ArmTracker::default();
    tracker.observe(&latest);
    assert_eq!(latest.wrong_sources, 2);
    assert!(tracker.report().is_none());
    assert!(landed_state_sample(&latest, start).is_none());
}
