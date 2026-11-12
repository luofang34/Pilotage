//! The height and the airspeed a GNSS-denied flight controller still
//! states: each reaches the sample under its own authorization bit, from
//! the decoded messages through the link cache.

use std::sync::{Arc, Mutex};
use web_time::Instant;

use pilotage_mavlink::codec::{FcMessage, FrameSource, VfrHud};
use pilotage_mavlink::link::{LinkState, apply_messages_at};
use pilotage_protocol::VehicleId;

use super::super::mavlink_batch;

const FC: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};
const HEIGHT: u32 = 0b01_0000;
const AIRSPEED: u32 = 0b10_0000;

/// An attitude, a local position 120 m up, an estimator status of
/// `valid_flags` at good quality, and a VFR_HUD stating `airspeed`.
fn sample(valid_flags: u8, airspeed: Option<f32>) -> pilotage_adapter_api::AvionicsSample {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    let messages = [
        FcMessage::AviateEstimatorStatus {
            time_usec: 1_000_000,
            valid_flags,
            quality: 2,
        },
        FcMessage::AttitudeQuaternion {
            time_boot_ms: 1_000,
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
        },
        FcMessage::LocalPositionNed {
            time_boot_ms: 1_000,
            pos_ned_m: [0.0, 0.0, -120.0],
            vel_ned_mps: [0.0; 3],
        },
        FcMessage::VfrHud(VfrHud {
            indicated_airspeed_mps: airspeed,
        }),
    ]
    .map(|message| (FC, message));
    apply_messages_at(&state, &messages, 0, 0, now);
    let batch = mavlink_batch(VehicleId::new(1), &state, now);
    let [sample] = batch.samples.as_slice() else {
        panic!("one sample expected, got {}", batch.samples.len());
    };
    sample.avionics.expect("an avionics group")
}

#[test]
fn without_gnss_the_height_and_airspeed_keep_their_bits() {
    // Attitude, rates, height, airspeed; no position, no velocity.
    let avionics = sample(0b11_0011, Some(52.5));
    assert_eq!(avionics.valid_flags, 0b11_0011);
    assert_eq!(
        avionics.kinematics.map(|group| group.pos_ned_m[2]),
        Some(-120.0)
    );
    let airspeed = avionics.airspeed.expect("an airspeed group");
    assert_eq!(airspeed.indicated_mps, 52.5);
    assert_eq!(
        airspeed.stamp.role,
        pilotage_adapter_api::SourceRole::OperationalEstimate
    );
}

#[test]
fn an_airspeed_the_estimator_does_not_authorize_carries_no_bit() {
    let avionics = sample(0b01_0011, Some(52.5));
    assert_eq!(avionics.valid_flags & (HEIGHT | AIRSPEED), HEIGHT);
    assert!(
        avionics.airspeed.is_some(),
        "the value travels; its use is not authorized"
    );
}

#[test]
fn a_flight_controller_without_a_probe_states_no_airspeed() {
    let avionics = sample(0b11_0011, None);
    assert_eq!(avionics.airspeed, None);
    assert_eq!(avionics.valid_flags & AIRSPEED, 0);
}
