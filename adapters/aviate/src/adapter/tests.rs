#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    Disposition, MeasurementClock, MeasurementStamp, RejectReason, VehicleAdapter,
};
use pilotage_protocol::VehicleId;

use crate::link::{AttitudeUpdate, KinematicsUpdate, LatestAviate};

use super::AviateAdapter;

fn state_with(att_age: Duration, kin_age: Duration) -> Arc<Mutex<LatestAviate>> {
    let now = Instant::now();
    let attitude_stamp = MeasurementStamp {
        source_id: 1,
        source_epoch: 1,
        sequence: 10,
        acquired_at_ns: 5_000_000_000,
        clock: MeasurementClock::VehicleBoot,
    };
    let kinematics_stamp = MeasurementStamp {
        sequence: 5,
        ..attitude_stamp
    };
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
            time_boot_ms: 5000,
            stamp: attitude_stamp,
            received_at: now.checked_sub(att_age).unwrap_or(now),
        }),
        kinematics: Some(KinematicsUpdate {
            pos_ned_m: [10.0, 20.0, -30.0],
            vel_ned_mps: [3.0, 4.0, -1.0],
            time_boot_ms: 5000,
            stamp: kinematics_stamp,
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
    assert_eq!(
        avionics.attitude_stamp.map(|stamp| stamp.sequence),
        Some(10)
    );
    assert_eq!(
        avionics.kinematics_stamp.map(|stamp| stamp.sequence),
        Some(5)
    );
}

#[test]
fn stale_attitude_is_withheld_but_kinematics_still_flow() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::ZERO),
    );
    let batch = adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    let avionics = batch.samples[0].avionics.expect("kinematics attached");
    assert!(avionics.attitude_stamp.is_none());
    assert_eq!(
        avionics.kinematics_stamp.map(|stamp| stamp.sequence),
        Some(5)
    );
    assert_eq!(avionics.valid_flags, 0b1100);
}

#[test]
fn dead_link_withholds_the_whole_sample() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::from_secs(10)),
    );
    assert!(adapter.sample_telemetry().samples.is_empty());
}

fn flight_frame(
    axes: Vec<(pilotage_protocol::LogicalAxisId, f32)>,
    edges: Vec<(
        pilotage_protocol::LogicalButtonId,
        pilotage_protocol::ButtonEdge,
    )>,
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

#[test]
fn stick_frame_reaches_the_fc_as_a_velocity_setpoint() {
    use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId};

    // A fake FC: any UDP socket we can read the uplink's frames from.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    // Heading east (yaw 90°) so body-frame rotation is observable.
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(uplink);

    let caps = adapter.capabilities();
    assert_eq!(caps.vehicles[0].scopes.len(), 1, "flight scope advertised");
    assert_eq!(caps.vehicles[0].scopes[0].axes.len(), 4);

    // Arm edge (stick frames are suppressed for a beat afterward so the
    // FC's single command slot is not overwritten before its loop runs).
    let outcome = adapter.apply_control(&flight_frame(
        vec![],
        vec![(LogicalButtonId::new(super::ARM_BUTTON), ButtonEdge::Pressed)],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);

    let mut buf = [0u8; 128];
    // First datagram: the arm command (COMMAND_LONG 400, param1=1).
    let (n, _) = fc.recv_from(&mut buf).expect("arm frame");
    assert_eq!(buf[7], 76, "COMMAND_LONG id");
    assert_eq!(
        f32::from_le_bytes([buf[10], buf[11], buf[12], buf[13]]),
        1.0
    );
    assert!(n >= 45);

    // Past the post-arm quiet window: full forward stick + half climb,
    // streamed like the 30 Hz control loop. The slew limiter ramps the
    // command from zero, so assert the ramp rather than an instant step.
    std::thread::sleep(Duration::from_millis(200));
    let f = |buf: &[u8; 128], off: usize| {
        f32::from_le_bytes([buf[10 + off], buf[11 + off], buf[12 + off], buf[13 + off]])
    };
    let mut last_ve = 0.0f32;
    for _ in 0..30 {
        let outcome = adapter.apply_control(&flight_frame(
            vec![
                (LogicalAxisId::new(super::PITCH_AXIS), 1.0),
                (LogicalAxisId::new(super::THROTTLE_AXIS), 0.5),
            ],
            vec![],
        ));
        assert_eq!(outcome.disposition, Disposition::Accepted);
        fc.recv_from(&mut buf).expect("setpoint frame");
        assert_eq!(buf[7], 84, "SET_POSITION_TARGET id");
        let ve = f(&buf, 20);
        assert!(
            ve >= last_ve - 1e-4,
            "ramp must not reverse: {ve} < {last_ve}"
        );
        last_ve = ve;
        std::thread::sleep(Duration::from_millis(20));
    }
    // Heading is east (yaw 90°), stick full-forward → velocity is +east:
    // vn≈0, ve ramping toward 3.
    let (vn, vz) = (f(&buf, 16), f(&buf, 24));
    assert!(vn.abs() < 0.05, "vn {vn}");
    assert!(last_ve > 1.0, "ramped east velocity, got {last_ve}");
    assert!(vz < -0.2, "climb demand present, got {vz}");
    let type_mask = u16::from_le_bytes([buf[10 + 48], buf[11 + 48]]);
    assert_eq!(type_mask, 2503);

    // Centered sticks switch to position-hold (DJI brake-then-hold):
    // the hold loop runs on the FC, so the ground streams a
    // position-valid setpoint (mask 2552) at the captured hold point
    // — the fake pose's NED position — with no ground-side gains.
    let outcome = adapter.apply_control(&flight_frame(
        vec![
            (LogicalAxisId::new(super::PITCH_AXIS), 0.0),
            (LogicalAxisId::new(super::THROTTLE_AXIS), 0.0),
        ],
        vec![],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    fc.recv_from(&mut buf).expect("hold frame");
    let hold_mask = u16::from_le_bytes([buf[10 + 48], buf[11 + 48]]);
    assert_eq!(hold_mask, 2552, "hold streams FC position mode");
    // Position fields carry the captured point (fake pose 10, 20, -30).
    assert!((f(&buf, 4) - 10.0).abs() < 1e-3, "hold north");
    assert!((f(&buf, 8) - 20.0).abs() < 1e-3, "hold east");
    assert!((f(&buf, 12) + 30.0).abs() < 1e-3, "hold down");
}

#[test]
fn control_frames_are_rejected_at_the_boundary() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    );
    let caps = adapter.capabilities();
    assert!(
        caps.vehicles[0].scopes.is_empty(),
        "telemetry-only without an uplink"
    );

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
