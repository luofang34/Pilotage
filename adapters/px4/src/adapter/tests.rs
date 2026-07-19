#![allow(clippy::expect_used, clippy::panic)]

//! Adapter-boundary behavior: the offboard arm sequence ordering on
//! the wire, sampling authorization from the standard status, and the
//! same gate discipline the Aviate adapter carries.

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, VehicleId};

use pilotage_mavlink::codec::{FcMessage, FrameSource};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{AuthorizationSource, LinkState};

use super::{ARM_BUTTON, DISARM_BUTTON, Px4Adapter, RESET_BUTTON, THROTTLE_AXIS};
use crate::uplink::Px4Uplink;

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn live_state() -> Arc<Mutex<LinkState>> {
    live_state_at(Instant::now()).0
}

fn live_state_at(start: Instant) -> (Arc<Mutex<LinkState>>, Instant) {
    let state = Arc::new(Mutex::new(LinkState {
        authorization_source: AuthorizationSource::StandardEstimatorStatus,
        reset_policy: pilotage_mavlink::ResetPolicy::SimulatorHeuristic,
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    }));
    feed_at(&state, 60_000, start);
    (state, start)
}

/// Feeds an authorized status + attitude + kinematics trio at the
/// given boot time, the way a live PX4 stream populates the cache.
fn feed_at(state: &Arc<Mutex<LinkState>>, time_boot_ms: u32, now: Instant) {
    let messages = [
        (
            SOURCE,
            FcMessage::EstimatorStatus {
                time_usec: u64::from(time_boot_ms) * 1_000,
                flags: 1 | 2 | 4 | 8 | 32,
            },
        ),
        (
            SOURCE,
            FcMessage::AttitudeQuaternion {
                time_boot_ms,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
        ),
        (
            SOURCE,
            FcMessage::LocalPositionNed {
                time_boot_ms,
                pos_ned_m: [1.0, 2.0, -3.0],
                vel_ned_mps: [0.0; 3],
            },
        ),
    ];
    apply_messages_at(state, &messages, 0, 0, now);
}

fn fake_fc() -> (UdpSocket, std::net::SocketAddr) {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let addr = fc.local_addr().expect("addr");
    (fc, addr)
}

fn uplink_to(addr: std::net::SocketAddr) -> Px4Uplink {
    Px4Uplink::new(addr).expect("uplink")
}

fn press(button: u16) -> pilotage_protocol::ScopedControlFrame {
    frame(
        vec![],
        vec![(LogicalButtonId::new(button), ButtonEdge::Pressed)],
    )
}

/// Neutral demonstration: every declared axis REPORTED at center — an
/// empty payload demonstrates nothing and must not clear a latch.
fn neutral() -> pilotage_protocol::ScopedControlFrame {
    frame(
        vec![
            (LogicalAxisId::new(super::ROLL_AXIS), 0.0),
            (LogicalAxisId::new(super::PITCH_AXIS), 0.0),
            (LogicalAxisId::new(THROTTLE_AXIS), 0.0),
            (LogicalAxisId::new(super::YAW_AXIS), 0.0),
        ],
        vec![],
    )
}

/// Drives a REAL source-epoch advance through the production message
/// path: the old stream goes silent, a low boot clock is quarantined,
/// dwells, and confirms — exactly what a restarted PX4 looks like.
fn drive_epoch_advance(state: &Arc<Mutex<LinkState>>, start: Instant) {
    let status = |tbm: u32| FcMessage::EstimatorStatus {
        time_usec: u64::from(tbm) * 1_000,
        flags: 1 | 2 | 4 | 8 | 32,
    };
    // Quarantined candidate after the silence budget, then source and
    // receive dwell, then confirmation.
    apply_messages_at(
        state,
        &[(SOURCE, status(1_000))],
        0,
        0,
        start + Duration::from_secs(4),
    );
    apply_messages_at(
        state,
        &[(SOURCE, status(1_400))],
        0,
        0,
        start + Duration::from_millis(4_400),
    );
    assert_eq!(
        state.lock().expect("state").source_epoch,
        2,
        "the reset heuristic must confirm a fresh epoch"
    );
    // The restarted stream repopulates the cleared cache.
    let trio = [
        (SOURCE, status(1_500)),
        (
            SOURCE,
            FcMessage::AttitudeQuaternion {
                time_boot_ms: 1_500,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
        ),
        (
            SOURCE,
            FcMessage::LocalPositionNed {
                time_boot_ms: 1_500,
                pos_ned_m: [1.0, 2.0, -3.0],
                vel_ned_mps: [0.0; 3],
            },
        ),
    ];
    apply_messages_at(state, &trio, 0, 0, start + Duration::from_millis(4_500));
}

fn frame(
    axes: Vec<(LogicalAxisId, f32)>,
    edges: Vec<(LogicalButtonId, ButtonEdge)>,
) -> pilotage_protocol::ScopedControlFrame {
    pilotage_protocol::ScopedControlFrame {
        session: pilotage_protocol::SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: pilotage_protocol::ScopeId::new(super::FLIGHT_SCOPE),
        generation: pilotage_protocol::Generation::new(1),
        sequence: pilotage_protocol::SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: pilotage_protocol::ControlPayload { axes, edges },
    }
}

/// Decodes a received frame into its message id, plus the MAV_CMD id
/// when the frame is a COMMAND_LONG.
fn received_kind(fc: &UdpSocket) -> (u32, Option<u16>) {
    let mut buf = [0u8; 128];
    let (len, _) = fc.recv_from(&mut buf).expect("datagram");
    assert!(len > 10, "runt frame");
    let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
    let command = (msg_id == 76).then(|| u16::from_le_bytes([buf[10 + 28], buf[10 + 29]]));
    (msg_id, command)
}

#[test]
fn arm_sequence_streams_before_offboard_and_arm() {
    let (fc, addr) = fake_fc();
    let mut uplink = uplink_to(addr);
    uplink.use_manual_clock();
    uplink.begin_arm(0.0);
    // The stream starts immediately with a zero-velocity setpoint.
    let (msg_id, _) = received_kind(&fc);
    assert_eq!(msg_id, 84, "setpoint stream precedes any command");
    assert!(uplink.streaming());

    // Before the warmup elapses, maintenance emits only heartbeat and
    // keepalive setpoints — never DO_SET_MODE or arm.
    uplink.advance_clock(Duration::from_millis(100));
    uplink.maintain();
    loop {
        let (msg_id, command) = received_kind(&fc);
        assert!(
            command != Some(176) && command != Some(400),
            "no mode/arm before warmup"
        );
        if msg_id == 84 {
            break;
        }
    }

    // After the warmup, DO_SET_MODE (OFFBOARD) then arm, in order.
    uplink.advance_clock(Duration::from_millis(300));
    uplink.maintain();
    let mut commands = vec![];
    while commands.len() < 2 {
        let (_, command) = received_kind(&fc);
        if let Some(command) = command {
            commands.push(command);
        }
    }
    assert_eq!(commands, vec![176, 400], "OFFBOARD precedes arm");
}

#[test]
fn disarm_stops_the_stream() {
    let (fc, addr) = fake_fc();
    let mut uplink = uplink_to(addr);
    uplink.begin_arm(0.0);
    uplink.send_disarm();
    assert!(!uplink.streaming(), "disarm stops the stream");
    let mut disarmed = false;
    for _ in 0..3 {
        let (msg_id, command) = received_kind(&fc);
        if msg_id == 76 && command == Some(400) {
            disarmed = true;
            break;
        }
    }
    assert!(disarmed, "disarm command reached the FC");
}

#[test]
fn sticks_are_ignored_until_an_arm_sequence_starts() {
    let (fc, addr) = fake_fc();
    let mut uplink = uplink_to(addr);
    uplink.send_stick_frame(0.0, 0.5, 0.5, 0.0);
    fc.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    let mut buf = [0u8; 128];
    assert!(
        fc.recv_from(&mut buf).is_err(),
        "no setpoint may leave before an explicit arm"
    );
}

#[test]
fn neutralize_sends_zero_velocity_and_stops() {
    let (fc, addr) = fake_fc();
    let mut uplink = uplink_to(addr);
    uplink.begin_arm(0.0);
    let _ = received_kind(&fc);
    uplink.neutralize();
    assert!(!uplink.streaming());
    let (msg_id, _) = received_kind(&fc);
    assert_eq!(msg_id, 84, "one final zero-velocity setpoint");
}

#[test]
fn sampled_telemetry_authorizes_from_the_standard_status() {
    let state = live_state();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    let batch = adapter.sample_telemetry();
    let sample = &batch.samples[0];
    let avionics = sample.avionics.expect("avionics");
    assert_eq!(avionics.valid_flags, 0b1111);
    assert!(sample.pose.is_some(), "authorized pair projects a pose");
    let pose = sample.pose.expect("pose");
    assert!((pose.x - 1.0).abs() < 1e-9 && (pose.y - 2.0).abs() < 1e-9);
}

#[test]
fn reset_press_latches_and_disarm_bypasses() {
    let (fc, addr) = fake_fc();
    let state = live_state();
    let mut adapter =
        Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink_to(addr));
    let outcome = adapter.apply_control(&press(RESET_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress)
    );
    assert_eq!(adapter.reset_spawns, 1);

    let outcome = adapter.apply_control(&press(ARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "arm against pre-reset measurements is rejected"
    );
    let outcome = adapter.apply_control(&press(DISARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "disarm bypasses the commanded-reset latch"
    );
    let mut buf = [0u8; 128];
    fc.recv_from(&mut buf).expect("disarm datagram");
}

#[test]
fn fresh_epoch_and_neutral_input_clear_the_reset_latch() {
    let start = Instant::now();
    let (state, start) = live_state_at(start);
    let fc = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let uplink = Px4Uplink::new(fc.local_addr().expect("addr")).expect("uplink");
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink);
    adapter.apply_control(&press(RESET_BUTTON));

    // The restarted PX4's stream is what advances the epoch — driven
    // through the production message path, never by poking counters.
    drive_epoch_advance(&state, start);

    // An arm edge is not neutral: clearing on it would let the very
    // frame that re-arms ride in before sticks were ever released.
    let outcome = adapter.apply_control(&press(ARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "an arm edge does not clear the latch"
    );
    // A deflected stick is not neutral, and neither is a payload that
    // omits declared axes — partial coverage demonstrates nothing.
    let deflected = frame(vec![(LogicalAxisId::new(THROTTLE_AXIS), 0.6)], vec![]);
    let outcome = adapter.apply_control(&deflected);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "deflected sticks do not clear the latch"
    );
    let empty = frame(vec![], vec![]);
    let outcome = adapter.apply_control(&empty);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "an EMPTY payload must not count as a neutral demonstration"
    );

    let outcome = adapter.apply_control(&neutral());
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "a full-axis neutral frame over the fresh stream clears the latch"
    );
    let outcome = adapter.apply_control(&press(ARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "arm proceeds once the latch has cleared"
    );
    let mut buf = [0u8; 128];
    fc.recv_from(&mut buf).expect("arm datagram reaches the FC");
}
