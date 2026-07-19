//! The commanded-reset latch at the adapter boundary: a reset request
//! invalidates every cached measurement's authority to validate control.
//! Until the estimate stream provably restarts (a fresh source epoch)
//! AND the holder demonstrates neutral input, everything except disarm
//! is rejected with a typed reason — an arm validated against pre-reset
//! data can reach the rebooting FC while its estimator is unconverged
//! and bank the vehicle on the ground.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, VehicleId};

use crate::link::{LatestAviate, ResetPolicy, apply_messages_at};
use crate::mavlink::{AviateMessage, FrameSource};

use super::super::{
    ARM_BUTTON, AviateAdapter, DISARM_BUTTON, PITCH_AXIS, RESET_BUTTON, ROLL_AXIS, THROTTLE_AXIS,
    YAW_AXIS,
};
use super::fixtures::{flight_frame, state_with};

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn adapter_with_fake_fc() -> (AviateAdapter, std::net::UdpSocket, Arc<Mutex<LatestAviate>>) {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    let state = state_with(Duration::ZERO, Duration::ZERO);
    let adapter = AviateAdapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink);
    (adapter, fc, state)
}

fn press(button: u16) -> pilotage_protocol::ScopedControlFrame {
    flight_frame(
        vec![],
        vec![(LogicalButtonId::new(button), ButtonEdge::Pressed)],
    )
}

fn neutral() -> pilotage_protocol::ScopedControlFrame {
    flight_frame(
        vec![
            (LogicalAxisId::new(ROLL_AXIS), 0.0),
            (LogicalAxisId::new(PITCH_AXIS), 0.0),
            (LogicalAxisId::new(THROTTLE_AXIS), 0.0),
            (LogicalAxisId::new(YAW_AXIS), 0.0),
        ],
        vec![],
    )
}

fn status(time_boot_ms: u32) -> AviateMessage {
    AviateMessage::AviateEstimatorStatus {
        time_usec: u64::from(time_boot_ms).saturating_mul(1_000),
        valid_flags: 0x0f,
        quality: 2,
    }
}

fn attitude(time_boot_ms: u32) -> AviateMessage {
    AviateMessage::AttitudeQuaternion {
        time_boot_ms,
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        rates_rps: [0.0; 3],
    }
}

fn kinematics(time_boot_ms: u32) -> AviateMessage {
    AviateMessage::LocalPositionNed {
        time_boot_ms,
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
    }
}

fn apply_at(state: &Arc<Mutex<LatestAviate>>, messages: &[AviateMessage], now: Instant) {
    let sourced = messages
        .iter()
        .copied()
        .map(|message| (SOURCE, message))
        .collect::<Vec<_>>();
    apply_messages_at(state, &sourced, 0, 0, now);
}

fn drive_fresh_epoch(state: &Arc<Mutex<LatestAviate>>) {
    let start = Instant::now();
    {
        let mut latest = state.lock().expect("state");
        latest.reset_policy = ResetPolicy::SimulatorHeuristic;
        latest.last_source_time_ms = Some(5_000);
        latest.last_accepted_at = start.checked_sub(Duration::from_secs(4));
    }
    apply_at(state, &[status(100)], start);
    apply_at(state, &[status(200)], start + Duration::from_millis(100));
    apply_at(
        state,
        &[status(400), attitude(400), kinematics(400)],
        start + Duration::from_millis(400),
    );

    let latest = state.lock().expect("state");
    assert_eq!(latest.source_epoch, 2, "link starts a fresh epoch");
    assert_eq!(
        latest.attitude.expect("attitude").stamp.source_epoch,
        2,
        "attitude is repopulated in the fresh epoch"
    );
    assert_eq!(
        latest.kinematics.expect("kinematics").stamp.source_epoch,
        2,
        "kinematics is repopulated in the fresh epoch"
    );
    assert_eq!(
        latest
            .estimator_status_stamp()
            .expect("status")
            .source_epoch,
        2,
        "authorization is repopulated in the fresh epoch"
    );
}

#[test]
fn reset_press_engages_the_latch_and_debounces_the_script() {
    let (mut adapter, _fc, _state) = adapter_with_fake_fc();
    let outcome = adapter.apply_control(&press(RESET_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "the reset press itself carries no control authority"
    );
    assert_eq!(adapter.reset_spawns, 1, "one script spawn recorded");
    let outcome = adapter.apply_control(&press(RESET_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress)
    );
    assert_eq!(
        adapter.reset_spawns, 1,
        "a second press inside the debounce window does not respawn"
    );
}

#[test]
fn stale_epoch_measurements_cannot_revalidate_arm_or_motion() {
    // The core hazard: the cached estimate is FRESH by age (received
    // moments ago) but belongs to the pre-reset FC — the source epoch
    // has not advanced. Arm and motion must stay rejected.
    let (mut adapter, _fc, _state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    let outcome = adapter.apply_control(&press(ARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "arm against pre-reset measurements is rejected"
    );
    let motion = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.8)], vec![]);
    let outcome = adapter.apply_control(&motion);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "motion against pre-reset measurements is rejected"
    );
}

#[test]
fn disarm_bypasses_the_latch() {
    let (mut adapter, fc, _state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    let outcome = adapter.apply_control(&press(DISARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "surrendering authority is never blocked by the reset latch"
    );
    let mut buf = [0u8; 128];
    fc.recv_from(&mut buf)
        .expect("disarm datagram reaches the FC");
}

#[test]
fn fresh_epoch_plus_neutral_input_clears_the_latch() {
    let (mut adapter, fc, state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    drive_fresh_epoch(&state);

    let outcome = adapter.apply_control(&neutral());
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "a neutral frame over the fresh stream clears the latch"
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

#[test]
fn source_epoch_counter_alone_cannot_clear_the_latch() {
    let (mut adapter, _fc, state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    state.lock().expect("state").source_epoch = 2;

    let outcome = adapter.apply_control(&neutral());
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "epoch-1 measurements cannot clear a latch whose counter says epoch 2"
    );
}

#[test]
fn fresh_epoch_requires_every_flight_axis() {
    let (mut adapter, _fc, state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    drive_fresh_epoch(&state);

    let missing_yaw = flight_frame(
        vec![
            (LogicalAxisId::new(ROLL_AXIS), 0.0),
            (LogicalAxisId::new(PITCH_AXIS), 0.0),
            (LogicalAxisId::new(THROTTLE_AXIS), 0.0),
        ],
        vec![],
    );
    let outcome = adapter.apply_control(&missing_yaw);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "a missing declared axis cannot clear the latch"
    );
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "an empty control payload cannot clear the latch"
    );
}

#[test]
fn fresh_epoch_with_active_input_stays_latched() {
    let (mut adapter, _fc, state) = adapter_with_fake_fc();
    adapter.apply_control(&press(RESET_BUTTON));
    drive_fresh_epoch(&state);

    // An arm edge is not neutral: clearing on it would let the very
    // frame that re-arms ride in before sticks were ever released.
    let outcome = adapter.apply_control(&press(ARM_BUTTON));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "an arm edge does not clear the latch"
    );
    // Deflected sticks are not neutral either.
    let deflected = flight_frame(
        vec![
            (LogicalAxisId::new(ROLL_AXIS), 0.0),
            (LogicalAxisId::new(PITCH_AXIS), 0.0),
            (LogicalAxisId::new(THROTTLE_AXIS), 0.5),
            (LogicalAxisId::new(YAW_AXIS), 0.0),
        ],
        vec![],
    );
    let outcome = adapter.apply_control(&deflected);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::ResetInProgress),
        "deflected sticks do not clear the latch"
    );
    let outcome = adapter.apply_control(&neutral());
    assert_eq!(outcome.disposition, Disposition::Accepted, "neutral clears");
}
