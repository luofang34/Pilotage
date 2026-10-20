use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{MeasurementClock, SourceIncarnation, SourceRole};

use crate::codec::{
    EXTENDED_SYS_STATE_ID, GLOBAL_POSITION_INT_ID, HEARTBEAT_ID, LOCAL_POSITION_NED_ID,
    encode_frame_v2_from,
};
use crate::link::{LinkState, ResetPolicy, apply_messages_at};
use crate::{LandedState, parse_datagram};

fn state(now: Instant) -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState {
        system_id: 23,
        component_id: 1,
        source_id: 42,
        source_incarnation: SourceIncarnation::new([7; 16]),
        started_at: now,
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    }))
}

fn frame(message_id: u32, payload: &[u8], crc_extra: u8, source: (u8, u8)) -> Vec<u8> {
    let mut bytes = vec![0; payload.len() + 12];
    let size = encode_frame_v2_from(
        0, message_id, payload, crc_extra, source.0, source.1, &mut bytes,
    );
    bytes.truncate(size);
    bytes
}

fn altitude(time_boot_ms: u32, millimetres: i32, source: (u8, u8)) -> Vec<u8> {
    let mut payload = [0; 28];
    payload[..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[12..16].copy_from_slice(&100_000_i32.to_le_bytes());
    payload[16..20].copy_from_slice(&millimetres.to_le_bytes());
    frame(GLOBAL_POSITION_INT_ID, &payload, 104, source)
}

fn landed(value: u8, source: (u8, u8)) -> Vec<u8> {
    frame(EXTENDED_SYS_STATE_ID, &[0, value], 130, source)
}

fn receive(state: &Arc<Mutex<LinkState>>, bytes: &[u8], now: Instant) {
    let mut decoded = Vec::new();
    let stats = parse_datagram(bytes, &mut decoded);
    assert_eq!(stats.unknown_ids, 0);
    apply_messages_at(state, &decoded, stats.crc_failures, stats.unknown_ids, now);
}

#[test]
fn home_relative_altitude_keeps_source_acquisition_and_receive_clocks() {
    let start = Instant::now();
    let state = state(start);
    let received = start + Duration::from_millis(75);
    receive(&state, &altitude(1_234, -2_750, (23, 1)), received);
    let latest = state.lock().expect("cache");
    let report = latest
        .relative_altitude_cached(received, Duration::ZERO)
        .expect("fresh home-relative altitude");
    assert_eq!(report.altitude_m, -2.75);
    assert_eq!(report.time_boot_ms, 1_234);
    assert_eq!(report.stamp.acquired_at_ns, 1_234_000_000);
    assert_eq!(report.stamp.clock, MeasurementClock::VehicleBoot);
    assert_eq!(report.stamp.source_id, 42);
    assert_eq!(
        report.stamp.source_incarnation,
        SourceIncarnation::new([7; 16])
    );
    assert_eq!(report.stamp.source_epoch, latest.source_epoch);
    assert_eq!(report.stamp.sequence, 0);
    assert_eq!(report.received_at, received);
    assert!(latest.kinematics.is_none());
    assert!(latest.gnss_fix.is_none());
    assert!(latest.estimator_status.is_none());
}

#[test]
fn unstamped_landed_report_uses_receive_acquisition_and_unknown_revokes_state() {
    let start = Instant::now();
    let state = state(start);
    let received = start + Duration::from_millis(25);
    receive(&state, &landed(1, (23, 1)), received);
    {
        let latest = state.lock().expect("cache");
        let report = latest
            .landed_state_cached(received, Duration::ZERO)
            .expect("fresh landed state");
        assert_eq!(report.state, LandedState::OnGround);
        assert_eq!(report.stamp.role, SourceRole::FcState);
        assert_eq!(report.stamp.clock, MeasurementClock::HostMonotonic);
        assert_eq!(report.stamp.acquired_at_ns, 25_000_000);
        assert_eq!(report.stamp.source_epoch, 1);
    }
    receive(&state, &landed(255, (23, 1)), received);
    let latest = state.lock().expect("cache");
    let report = latest.fc_reports.landed_state.expect("unknown report");
    assert_eq!(report.state, LandedState::Undefined);
    assert_eq!(report.stamp.sequence, 1);
}

#[test]
fn only_the_selected_flight_controller_can_supply_flight_procedure_evidence() {
    let now = Instant::now();
    let state = state(now);
    for source in [(24, 1), (23, 191)] {
        receive(&state, &altitude(1_000, 4_000, source), now);
        receive(&state, &landed(2, source), now);
    }
    let latest = state.lock().expect("cache");
    assert!(latest.fc_reports.relative_altitude.is_none());
    assert!(latest.fc_reports.landed_state.is_none());
    assert_eq!(latest.wrong_sources, 4);
    assert!(latest.last_source_time_ms.is_none());
}

#[test]
fn duplicate_and_older_altitude_cannot_refresh_or_replace_observed_progress() {
    let now = Instant::now();
    let state = state(now);
    receive(&state, &altitude(1_000, 0, (23, 1)), now);
    receive(&state, &landed(1, (23, 1)), now);
    let later = now + Duration::from_secs(1);
    receive(&state, &altitude(1_000, 3_000, (23, 1)), later);
    receive(&state, &altitude(999, 4_000, (23, 1)), later);
    let latest = state.lock().expect("cache");
    let report = latest.fc_reports.relative_altitude.expect("initial report");
    assert_eq!(report.altitude_m, 0.0);
    assert_eq!(report.received_at, now);
    assert!(
        latest
            .relative_altitude_cached(later, Duration::from_millis(999))
            .is_none()
    );
    assert!(
        latest
            .landed_state_cached(later, Duration::from_millis(999))
            .is_none()
    );
    assert_eq!(latest.duplicate_measurements, 1);
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn future_receive_instants_do_not_satisfy_freshness() {
    let now = Instant::now();
    let state = state(now);
    let future = now + Duration::from_millis(1);
    receive(&state, &altitude(1_000, 1_000, (23, 1)), future);
    receive(&state, &landed(2, (23, 1)), future);
    let latest = state.lock().expect("cache");
    assert!(
        latest
            .relative_altitude_cached(now, Duration::MAX)
            .is_none()
    );
    assert!(latest.landed_state_cached(now, Duration::MAX).is_none());
}

#[test]
fn confirmed_source_reset_clears_landed_evidence_and_restamps_relative_altitude() {
    let now = Instant::now();
    let state = state(now);
    state.lock().expect("cache").reset_policy = ResetPolicy::SimulatorHeuristic;
    receive(&state, &altitude(10_000, 3_000, (23, 1)), now);
    receive(&state, &landed(2, (23, 1)), now);
    let candidate_at = now + Duration::from_secs(3);
    receive(&state, &altitude(100, 0, (23, 1)), candidate_at);
    {
        let latest = state.lock().expect("cache");
        assert!(latest.pending_reset.is_some());
        assert!(
            latest
                .relative_altitude_cached(candidate_at, Duration::MAX)
                .is_none()
        );
        assert!(
            latest
                .landed_state_cached(candidate_at, Duration::MAX)
                .is_none()
        );
    }
    let confirmed_at = candidate_at + Duration::from_millis(250);
    receive(&state, &altitude(350, 200, (23, 1)), confirmed_at);
    let latest = state.lock().expect("cache");
    let report = latest
        .relative_altitude_cached(confirmed_at, Duration::ZERO)
        .expect("new epoch altitude");
    assert_eq!(report.stamp.source_epoch, 2);
    assert_eq!(report.stamp.sequence, 0);
    assert_eq!(report.altitude_m, 0.2);
    assert!(latest.fc_reports.landed_state.is_none());
}

#[test]
fn altitude_clock_wrap_starts_an_epoch_without_reusing_landed_evidence() {
    let now = Instant::now();
    let state = state(now);
    receive(&state, &altitude(u32::MAX - 100, 3_000, (23, 1)), now);
    receive(&state, &landed(2, (23, 1)), now);
    receive(&state, &altitude(100, 3_200, (23, 1)), now);
    let latest = state.lock().expect("cache");
    let report = latest.fc_reports.relative_altitude.expect("wrapped clock");
    assert_eq!(report.stamp.source_epoch, 2);
    assert_eq!(report.stamp.sequence, 0);
    assert!(latest.fc_reports.landed_state.is_none());
}

#[test]
fn local_ned_height_does_not_supply_home_relative_altitude() {
    let now = Instant::now();
    let state = state(now);
    let mut payload = [0_u8; 28];
    payload[..4].copy_from_slice(&1_000_u32.to_le_bytes());
    payload[12..16].copy_from_slice(&(-3.0_f32).to_le_bytes());
    receive(
        &state,
        &frame(LOCAL_POSITION_NED_ID, &payload, 185, (23, 1)),
        now,
    );
    let latest = state.lock().expect("cache");
    assert!(latest.kinematics.is_some());
    assert!(latest.fc_reports.relative_altitude.is_none());
    assert!(latest.fc_reports.landed_state.is_none());
}

#[test]
fn a_reset_detected_by_another_measurement_group_also_clears_flight_reports() {
    let now = Instant::now();
    let state = state(now);
    receive(&state, &altitude(u32::MAX - 100, 3_000, (23, 1)), now);
    receive(&state, &landed(2, (23, 1)), now);
    receive(
        &state,
        &frame(HEARTBEAT_ID, &[4, 0, 0, 0, 2, 3, 0x80, 4, 3], 50, (23, 1)),
        now,
    );
    {
        let latest = state.lock().expect("cache");
        assert_eq!(latest.heartbeat_armed, Some(true));
        assert_eq!(latest.fc_reports.custom_mode, Some(4));
    }
    let mut payload = [0_u8; 28];
    payload[..4].copy_from_slice(&100_u32.to_le_bytes());
    receive(
        &state,
        &frame(LOCAL_POSITION_NED_ID, &payload, 185, (23, 1)),
        now,
    );
    let latest = state.lock().expect("cache");
    assert_eq!(latest.source_epoch, 2);
    assert!(latest.fc_reports.relative_altitude.is_none());
    assert!(latest.fc_reports.landed_state.is_none());
    assert!(latest.last_heartbeat.is_none());
    assert!(latest.heartbeat_armed.is_none());
    assert!(latest.fc_reports.custom_mode.is_none());
    assert!(latest.fc_reports.autopilot.is_none());
}

#[test]
fn slower_altitude_stream_can_join_within_the_configured_inter_group_lag() {
    let now = Instant::now();
    let state = state(now);
    let mut payload = [0_u8; 28];
    payload[..4].copy_from_slice(&1_200_u32.to_le_bytes());
    receive(
        &state,
        &frame(LOCAL_POSITION_NED_ID, &payload, 185, (23, 1)),
        now,
    );
    receive(&state, &altitude(1_000, 2_000, (23, 1)), now);
    let latest = state.lock().expect("cache");
    let report = latest
        .relative_altitude_cached(now, Duration::ZERO)
        .expect("independent slower altitude");
    assert_eq!(report.altitude_m, 2.0);
    assert_eq!(report.time_boot_ms, 1_000);
    assert_eq!(latest.last_source_time_ms, Some(1_200));
}

#[test]
fn each_landed_state_retains_its_distinct_meaning() {
    let now = Instant::now();
    let state = state(now);
    for (wire, expected) in [
        (0, LandedState::Undefined),
        (1, LandedState::OnGround),
        (2, LandedState::InAir),
        (3, LandedState::Takeoff),
        (4, LandedState::Landing),
    ] {
        receive(&state, &landed(wire, (23, 1)), now);
        assert_eq!(
            state
                .lock()
                .expect("cache")
                .landed_state_cached(now, Duration::ZERO)
                .expect("landed report")
                .state,
            expected
        );
    }
}

#[test]
fn zero_truncated_reports_preserve_zero_altitude_and_unknown_landed_state() {
    let now = Instant::now();
    let state = state(now);
    receive(
        &state,
        &frame(GLOBAL_POSITION_INT_ID, &[1], 104, (23, 1)),
        now,
    );
    receive(
        &state,
        &frame(EXTENDED_SYS_STATE_ID, &[0], 130, (23, 1)),
        now,
    );
    let latest = state.lock().expect("cache");
    assert_eq!(
        latest
            .fc_reports
            .relative_altitude
            .expect("altitude")
            .altitude_m,
        0.0
    );
    assert_eq!(
        latest.fc_reports.landed_state.expect("landed").state,
        LandedState::Undefined
    );
}

#[test]
fn corrupted_flight_reports_do_not_refresh_the_cache() {
    let now = Instant::now();
    let state = state(now);
    for mut bytes in [altitude(1_000, 3_000, (23, 1)), landed(2, (23, 1))] {
        bytes[10] ^= 1;
        receive(&state, &bytes, now);
    }
    let latest = state.lock().expect("cache");
    assert_eq!(latest.crc_failures, 2);
    assert!(latest.fc_reports.relative_altitude.is_none());
    assert!(latest.fc_reports.landed_state.is_none());
}
