//! The adapter half of estimator-status authorization: sampled telemetry
//! tracks a status-only revocation and never re-grants cached numerics.
//! The link-layer half of this contract lives with `pilotage_mavlink`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_mavlink::LinkState;
use pilotage_mavlink::codec::{FcMessage, FrameSource};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::link::estimator::{QUALITY_GOOD, QUALITY_UNUSABLE};
use pilotage_protocol::VehicleId;

use super::super::AviateAdapter;

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn apply(state: &Arc<Mutex<LinkState>>, messages: &[FcMessage]) {
    let messages = messages
        .iter()
        .copied()
        .map(|message| (SOURCE, message))
        .collect::<Vec<_>>();
    apply_messages_at(state, &messages, 0, 0, Instant::now());
}

fn private_status(time_usec: u64, valid_flags: u8, quality: u8) -> FcMessage {
    FcMessage::AviateEstimatorStatus {
        time_usec,
        valid_flags,
        quality,
    }
}

fn attitude(time_boot_ms: u32) -> FcMessage {
    FcMessage::AttitudeQuaternion {
        time_boot_ms,
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        rates_rps: [0.0; 3],
    }
}

fn kinematics(time_boot_ms: u32) -> FcMessage {
    FcMessage::LocalPositionNed {
        time_boot_ms,
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
    }
}

#[test]
fn sampled_telemetry_tracks_status_only_revocation_without_regrant() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    apply(&state, &[attitude(100), kinematics(100)]);
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state.clone());

    let initial = adapter.sample_telemetry();
    let sample = &initial.samples[0];
    let avionics = sample.avionics.expect("avionics");
    assert_eq!(
        (avionics.valid_flags, avionics.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert!(avionics.estimator_status_stamp.is_none());
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());

    apply(
        &state,
        &[
            private_status(200_000, 0x0f, 2),
            attitude(200),
            kinematics(200),
        ],
    );
    let authorized = adapter.sample_telemetry();
    let sample = &authorized.samples[0];
    let avionics = sample.avionics.expect("avionics");
    assert_eq!(
        (avionics.valid_flags, avionics.quality),
        (0x0f, QUALITY_GOOD)
    );
    assert_eq!(
        avionics.estimator_status_stamp.map(|stamp| stamp.sequence),
        Some(0)
    );
    assert!(sample.pose.is_some());
    assert!(sample.speed.is_some());

    apply(&state, &[private_status(300_000, 0, 0)]);
    apply(&state, &[private_status(400_000, 0x0f, 2)]);
    let revoked = adapter.sample_telemetry();
    let sample = &revoked.samples[0];
    let avionics = sample.avionics.expect("avionics");
    assert_eq!(
        (avionics.valid_flags, avionics.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert_eq!(
        avionics.estimator_status_stamp.map(|stamp| stamp.sequence),
        Some(2)
    );
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
}
