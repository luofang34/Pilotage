#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, MeasurementClock, MeasurementStamp, RejectReason, SourceIncarnation, SourceRole,
    VehicleAdapter,
};
use pilotage_protocol::{ButtonEdge, LogicalButtonId, VehicleId};

use pilotage_mavlink::link::KinematicsUpdate;

mod fixtures;
mod flight_control;
mod source_roles;
use fixtures::{flight_frame, state_with, state_with_acquisition_skew};

use super::{AviateAdapter, sampling::measurement_pair_is_coherent};

#[test]
fn measurement_pair_requires_full_identity_clock_and_bounded_skew() {
    let state = state_with_acquisition_skew(Duration::ZERO, Duration::ZERO, 300_000_000);
    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("attitude");
    let kinematics = latest.kinematics.expect("kinematics");
    assert!(measurement_pair_is_coherent(
        attitude,
        kinematics,
        latest.maximum_inter_group_skew_ms
    ));

    for stamp in [
        MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            source_id: 2,
            ..kinematics.stamp
        },
        MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            source_incarnation: SourceIncarnation::new([2; 16]),
            ..kinematics.stamp
        },
        MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            source_epoch: 2,
            ..kinematics.stamp
        },
        MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            clock: MeasurementClock::Simulation,
            ..kinematics.stamp
        },
    ] {
        assert!(!measurement_pair_is_coherent(
            attitude,
            KinematicsUpdate {
                stamp,
                ..kinematics
            },
            latest.maximum_inter_group_skew_ms
        ));
    }
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
    let pose = sample.pose.expect("coherent planar pose");
    assert_eq!(pose.x, 10.0);
    assert_eq!(pose.y, 20.0);
    assert!((pose.heading - core::f64::consts::FRAC_PI_2).abs() < 1e-3);
    assert!((sample.speed.expect("coherent speed") - 5.0).abs() < 1e-6);
    let avionics = sample.avionics.expect("avionics attached");
    assert_eq!(
        avionics.kinematics.expect("kinematics").pos_ned_m,
        [10.0, 20.0, -30.0]
    );
    assert_eq!(avionics.valid_flags, 0b1111);
    assert_eq!(
        avionics.estimator_status_stamp.map(|stamp| stamp.sequence),
        Some(7)
    );
    assert_eq!(
        avionics.attitude.map(|group| group.stamp.sequence),
        Some(10)
    );
    assert_eq!(
        avionics.kinematics.map(|group| group.stamp.sequence),
        Some(5)
    );
}

#[test]
fn missing_status_normalizes_hand_built_numeric_groups_fail_closed() {
    let state = state_with(Duration::ZERO, Duration::ZERO);
    state.lock().expect("lock").estimator_status = None;
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state);
    let batch = adapter.sample_telemetry();
    let sample = &batch.samples[0];
    let avionics = sample.avionics.expect("avionics");
    assert_eq!((avionics.valid_flags, avionics.quality), (0, 2));
    assert!(avionics.estimator_status_stamp.is_none());
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
}

#[test]
fn unusable_group_with_diagnostic_flags_does_not_taint_an_authorized_group() {
    let state = state_with(Duration::ZERO, Duration::ZERO);
    {
        let mut latest = state.lock().expect("lock");
        let attitude = latest.attitude.as_mut().expect("attitude");
        attitude.valid_flags = 0b0011;
        attitude.quality = 2;
        let kinematics = latest.kinematics.as_mut().expect("kinematics");
        kinematics.valid_flags = 0b1100;
        kinematics.quality = 0;
    }
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state);
    let sample = adapter.sample_telemetry().samples.remove(0);
    let avionics = sample.avionics.expect("avionics");
    assert_eq!((avionics.valid_flags, avionics.quality), (0b1100, 0));
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
}

#[test]
fn over_skew_groups_flow_without_a_planar_projection() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with_acquisition_skew(Duration::ZERO, Duration::ZERO, 301_000_000),
    );
    let batch = adapter.sample_telemetry();
    let sample = batch.samples.first().expect("sample");
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
    let avionics = sample.avionics.expect("avionics");
    assert!(avionics.attitude.is_some());
    assert!(avionics.kinematics.is_some());
}

#[test]
fn stale_attitude_is_withheld_but_kinematics_still_flow() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::ZERO),
    );
    let batch = adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    let sample = &batch.samples[0];
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
    let avionics = sample.avionics.expect("kinematics attached");
    assert!(avionics.attitude.is_none());
    assert_eq!(
        avionics.kinematics.map(|group| group.stamp.sequence),
        Some(5)
    );
    assert_eq!(
        avionics.kinematics.expect("kinematics").vel_ned_mps,
        [3.0, 4.0, -1.0]
    );
    assert_eq!(avionics.valid_flags, 0b1100);
}

#[test]
fn stale_kinematics_is_withheld_but_attitude_still_flows() {
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::from_secs(10)),
    );
    let batch = adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    let sample = &batch.samples[0];
    assert!(sample.pose.is_none());
    assert!(sample.speed.is_none());
    let avionics = sample.avionics.expect("attitude attached");
    assert_eq!(
        avionics.attitude.map(|group| group.stamp.sequence),
        Some(10)
    );
    assert_eq!(
        avionics.attitude.expect("attitude").rates_rps,
        [0.0, 0.0, 0.1]
    );
    assert!(avionics.kinematics.is_none());
    assert_eq!(avionics.valid_flags, 0b0011);
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
fn incomplete_measurement_pair_cannot_seed_a_control_setpoint() {
    for (attitude_age, kinematics_age) in [
        (Duration::from_secs(10), Duration::ZERO),
        (Duration::ZERO, Duration::from_secs(10)),
    ] {
        let uplink = crate::uplink::FlightUplink::new().expect("uplink");
        let mut adapter =
            AviateAdapter::from_state(VehicleId::new(1), state_with(attitude_age, kinematics_age))
                .with_uplink(uplink);
        let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
        assert_eq!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::MeasurementUnavailable)
        );
    }
}

#[test]
fn over_skew_measurement_pair_cannot_seed_a_control_setpoint() {
    let uplink = crate::uplink::FlightUplink::new().expect("uplink");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with_acquisition_skew(Duration::ZERO, Duration::ZERO, 301_000_000),
    )
    .with_uplink(uplink);
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn estimator_revocation_cannot_seed_a_control_setpoint() {
    let state = state_with(Duration::ZERO, Duration::ZERO);
    {
        let mut latest = state.lock().expect("lock");
        latest.attitude.as_mut().expect("attitude").quality = 2;
        latest.kinematics.as_mut().expect("kinematics").quality = 2;
    }
    let uplink = crate::uplink::FlightUplink::new().expect("uplink");
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn disarm_does_not_require_a_current_measurement_pair() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::from_secs(10)),
    )
    .with_uplink(uplink);

    let outcome = adapter.apply_control(&flight_frame(
        vec![],
        vec![(
            LogicalButtonId::new(super::DISARM_BUTTON),
            ButtonEdge::Pressed,
        )],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let mut buf = [0_u8; 128];
    let len = fc.recv(&mut buf).expect("receive disarm");
    assert!(len >= 45);
    assert_eq!(buf[7], pilotage_mavlink::codec::COMMAND_LONG_ID as u8);
    assert_eq!(
        f32::from_le_bytes(buf[10..14].try_into().expect("param1")),
        0.0
    );
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
        action_ids: vec![],
        session: pilotage_protocol::SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: pilotage_protocol::ScopeId::new("vehicle.motion"),
        generation: pilotage_protocol::Generation::new(1),
        sequence: pilotage_protocol::SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: pilotage_protocol::ControlPayload {
            axes: vec![],
            edges: vec![],
        },
        intent: None,
        actions: vec![],
    };
    let outcome = adapter.apply_control(&frame);
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );
}

mod estimator_authorization;
mod link_loss_enact;
mod reset_latch;
