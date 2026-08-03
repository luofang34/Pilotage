#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::telemetry_message;

fn stamp(role: wire::SourceRole) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        role: role as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        source_id: 4,
        source_epoch: 1,
        sequence: 2,
        acquired_at_ns: 500,
        clock: wire::MeasurementClock::Simulation as i32,
        source_incarnation: vec![0x11; 16],
    }
}

fn empty_sample() -> wire::TelemetrySample {
    wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 1 }),
        tick: Some(wire::SimTick { value: 5 }),
        observed_at: Some(wire::MonoTimestamp { nanos: 900 }),
        pose: None,
        velocity: None,
        avionics: None,
        sim_truth: None,
        fc_state: None,
        gimbal: None,
        nav_guidance: None,
    }
}

#[test]
fn pose_absence_leaves_flattened_fields_absent() {
    let mut sample = empty_sample();
    sample.velocity = Some(wire::Velocity2d {
        linear_x_mps: 4.0,
        linear_y_mps: 0.0,
        angular_rad_s: 0.1,
    });
    let message = telemetry_message(sample);
    assert_eq!(message.vehicle_id, 1);
    assert!(message.pose.is_none());
    assert!(message.x_m.is_none());
    // Velocity present flattens through.
    assert_eq!(message.linear_x_mps, Some(4.0));
    assert_eq!(message.angular_rad_s, Some(0.1));
    assert!(message.avionics.is_none());
}

#[test]
fn every_stamped_group_the_sample_carries_reaches_the_message() {
    let mut sample = empty_sample();
    sample.gimbal = Some(Box::new(wire::GimbalAttitude {
        quat_w: 1.0,
        stamp: Some(stamp(wire::SourceRole::PayloadDevice)),
        ..Default::default()
    }));
    sample.nav_guidance = Some(Box::new(wire::NavGuidanceState {
        stamp: Some(stamp(wire::SourceRole::NavigationSolution)),
        to_ident: String::from("WP01"),
        ..Default::default()
    }));
    sample.fc_state = Some(Box::new(wire::FcState {
        arm_state: 2,
        stamp: Some(stamp(wire::SourceRole::FcState)),
        ..Default::default()
    }));
    let message = telemetry_message(sample);
    // A group the host encodes and this decode drops is invisible in the
    // browser with no error to notice it by, so each wiring is pinned.
    assert!(
        message.gimbal.is_some(),
        "gimbal lane must reach the viewer"
    );
    assert!(
        message.nav_guidance.is_some(),
        "guidance lane must reach the viewer"
    );
    assert!(message.fc_state.is_some(), "fc lane must reach the viewer");
}

#[test]
fn unstamped_groups_are_absent_rather_than_defaulted() {
    let mut sample = empty_sample();
    sample.gimbal = Some(Box::new(wire::GimbalAttitude {
        quat_w: 1.0,
        ..Default::default()
    }));
    sample.nav_guidance = Some(Box::new(wire::NavGuidanceState {
        to_ident: String::from("WP01"),
        ..Default::default()
    }));
    let message = telemetry_message(sample);
    assert!(message.gimbal.is_none());
    assert!(message.nav_guidance.is_none());
}
