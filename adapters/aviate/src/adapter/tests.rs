#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::VehicleId;

use crate::link::{AttitudeUpdate, KinematicsUpdate, LatestAviate};

use super::AviateAdapter;

fn state_with(att_age: Duration, kin_age: Duration) -> Arc<Mutex<LatestAviate>> {
    let now = Instant::now();
    let state = LatestAviate {
        attitude: Some(AttitudeUpdate {
            // 90° yaw: heading east.
            quat_wxyz: [
                core::f32::consts::FRAC_1_SQRT_2,
                0.0,
                0.0,
                core::f32::consts::FRAC_1_SQRT_2,
            ],
            rates_rps: [0.0, 0.0, 0.1],
            received_at: now.checked_sub(att_age).unwrap_or(now),
        }),
        kinematics: Some(KinematicsUpdate {
            pos_ned_m: [10.0, 20.0, -30.0],
            vel_ned_mps: [3.0, 4.0, -1.0],
            time_boot_ms: 5000,
            received_at: now.checked_sub(kin_age).unwrap_or(now),
        }),
        ..LatestAviate::default()
    };
    Arc::new(Mutex::new(state))
}

#[test]
fn fresh_state_samples_pose_speed_and_avionics() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    );
    let batch = adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    let sample = &batch.samples[0];
    assert_eq!(sample.pose.x, 10.0);
    assert_eq!(sample.pose.y, 20.0);
    assert!((sample.pose.heading - core::f64::consts::FRAC_PI_2).abs() < 1e-3);
    assert!((sample.speed - 5.0).abs() < 1e-6);
    let avionics = sample.avionics.expect("avionics attached");
    assert_eq!(avionics.pos_ned_m, [10.0, 20.0, -30.0]);
    assert_eq!(avionics.valid_flags, 0b1111);
}

#[test]
fn stale_attitude_is_withheld_but_kinematics_still_flow() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::ZERO),
    );
    let batch = adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    assert!(batch.samples[0].avionics.is_none());
}

#[test]
fn dead_link_withholds_the_whole_sample() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::from_secs(10)),
    );
    assert!(adapter.sample_telemetry().samples.is_empty());
}

#[test]
fn control_frames_are_rejected_at_the_boundary() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    );
    let caps = adapter.capabilities();
    assert!(caps.vehicles[0].scopes.is_empty(), "telemetry-only");

    let frame = pilotage_protocol::ScopedControlFrame {
        session: pilotage_protocol::SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: pilotage_protocol::ScopeId::new("vehicle.motion"),
        generation: pilotage_protocol::Generation::new(1),
        sequence: pilotage_protocol::SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: pilotage_protocol::ControlPayload {
            axes: vec![],
            edges: vec![],
        },
    };
    let outcome = adapter.apply_control(&frame);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );
}
