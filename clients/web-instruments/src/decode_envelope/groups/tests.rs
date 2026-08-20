#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::{avionics_message, gimbal_message, nav_guidance_message};
use crate::decode_envelope::stamp_message;

fn stamp(source_id: u64, incarnation: Vec<u8>) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        role: wire::SourceRole::OperationalEstimate as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        source_id,
        source_epoch: 3,
        sequence: 9,
        acquired_at_ns: 123,
        clock: wire::MeasurementClock::Simulation as i32,
        source_incarnation: incarnation,
    }
}

fn stamp_with_role(role: wire::SourceRole) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        role: role as i32,
        ..stamp(11, vec![0x5A; 16])
    }
}

#[allow(deprecated)]
fn full_avionics() -> wire::AvionicsState {
    wire::AvionicsState {
        baro_alt_m: 0.0,
        baro_stamp: None,
        quat_w: 1.0,
        quat_x: 0.2,
        quat_y: 0.3,
        quat_z: 0.4,
        rate_p_rad_s: 0.5,
        rate_q_rad_s: 0.6,
        rate_r_rad_s: 0.7,
        pos_n_m: 10.0,
        pos_e_m: 11.0,
        pos_d_m: -2.0,
        vel_n_mps: 1.0,
        vel_e_mps: 2.0,
        vel_d_mps: 3.0,
        valid_flags: 0x0f,
        quality: 0,
        arm_state: 1,
        attitude_stamp: Some(stamp(7, vec![0xAB; 16])),
        kinematics_stamp: Some(stamp(8, vec![0xCD; 16])),
        estimator_status_stamp: Some(stamp(9, vec![0xEF; 16])),
    }
}

fn guidance(stamp: Option<wire::MeasurementStamp>) -> wire::NavGuidanceState {
    wire::NavGuidanceState {
        stamp,
        to_ident: String::from("WP02"),
        from_ident: String::from("WP01"),
        course_rad: 1.25,
        lateral_deviation_m: -30.0,
        vertical_deviation_m: 12.5,
        distance_to_waypoint_m: 3704.0,
        leg_index: 2,
        waypoint_count: 5,
        solution_quality: 1,
    }
}

fn gimbal(stamp: Option<wire::MeasurementStamp>) -> wire::GimbalAttitude {
    wire::GimbalAttitude {
        quat_w: 1.0,
        quat_x: 0.0,
        quat_y: 0.25,
        quat_z: 0.0,
        rate_x_rad_s: 0.5,
        rate_y_rad_s: -0.5,
        rate_z_rad_s: 0.75,
        stamp,
        flags: 0b101,
        failure_flags: 0b10,
    }
}

#[test]
fn present_groups_flatten_and_mirror_their_nested_form() {
    let avionics = avionics_message(full_avionics());
    let attitude = avionics.attitude.expect("attitude present with its stamp");
    let quat = avionics.quat.expect("flat quat mirrors attitude");
    assert_eq!(quat.w, attitude.quat.w);
    assert_eq!(avionics.rates.expect("flat rates"), attitude.rates);
    let kinematics = avionics
        .kinematics
        .expect("kinematics present with its stamp");
    assert_eq!(avionics.pos_ned.expect("flat pos"), kinematics.pos_ned);
    assert_eq!(avionics.vel_ned.expect("flat vel"), kinematics.vel_ned);
}

#[test]
fn absent_stamp_zeroes_no_group() {
    let mut state = full_avionics();
    state.attitude_stamp = None;
    let avionics = avionics_message(state);
    // The proto3 quat defaults are not a measurement without an attitude
    // stamp: the group and its flattened mirror must be absent, not zero.
    assert!(avionics.attitude.is_none());
    assert!(avionics.quat.is_none());
    assert!(avionics.rates.is_none());
    // The kinematics group, whose stamp survives, is unaffected.
    assert!(avionics.kinematics.is_some());
}

#[test]
fn incarnation_is_hex_only_when_sixteen_bytes() {
    assert_eq!(
        stamp_message(stamp(1, vec![0xAB; 16]))
            .source_incarnation
            .as_deref(),
        Some("abababababababababababababababab")
    );
    assert!(
        stamp_message(stamp(1, vec![0xAB; 4]))
            .source_incarnation
            .is_none()
    );
    assert!(
        stamp_message(stamp(1, Vec::new()))
            .source_incarnation
            .is_none()
    );
}

#[test]
fn gimbal_needs_a_payload_device_stamp() {
    let device = gimbal_message(gimbal(Some(stamp_with_role(
        wire::SourceRole::PayloadDevice,
    ))))
    .expect("payload-device stamp admits the lane");
    assert_eq!(device.quat.y, 0.25);
    assert_eq!(device.rates_rad_s, [0.5, -0.5, 0.75]);
    assert_eq!(device.flags, 0b101);
    assert_eq!(device.failure_flags, 0b10);
    assert!(gimbal_message(gimbal(None)).is_none());
    // A device report wearing another lane's role is mislabeled: it must
    // never reach the camera view as an orientation.
    assert!(gimbal_message(gimbal(Some(stamp_with_role(wire::SourceRole::FcState)))).is_none());
}

#[test]
fn nav_guidance_needs_a_navigation_solution_stamp() {
    let nav = nav_guidance_message(guidance(Some(stamp_with_role(
        wire::SourceRole::NavigationSolution,
    ))))
    .expect("navigation-solution stamp admits the lane");
    assert_eq!(nav.to_ident, "WP02");
    assert_eq!(nav.from_ident, "WP01");
    assert_eq!(nav.course_rad, 1.25);
    assert_eq!(nav.lateral_deviation_m, -30.0);
    assert_eq!(nav.vertical_deviation_m, 12.5);
    assert_eq!(nav.distance_to_waypoint_m, 3704.0);
    assert_eq!(nav.leg_index, 2);
    assert_eq!(nav.waypoint_count, 5);
    assert_eq!(nav.solution_quality, 1);
    assert_eq!(nav.stamp.role, wire::SourceRole::NavigationSolution as i32);
    assert!(nav_guidance_message(guidance(None)).is_none());
    // Guidance is display context and never a fallback for a missing
    // estimate: an estimate-role lane is not a navigation solution.
    assert!(
        nav_guidance_message(guidance(Some(stamp_with_role(
            wire::SourceRole::OperationalEstimate
        ))))
        .is_none()
    );
}

#[test]
fn nav_guidance_carries_not_tracking_through_as_nan() {
    let mut state = guidance(Some(stamp_with_role(wire::SourceRole::NavigationSolution)));
    state.lateral_deviation_m = f32::NAN;
    state.vertical_deviation_m = f32::NAN;
    let nav = nav_guidance_message(state).expect("stamped guidance decodes");
    // Zero would read as on-course; the absence must survive the decode so
    // the display profile can remove the deviation instead of centering it.
    assert!(nav.lateral_deviation_m.is_nan());
    assert!(nav.vertical_deviation_m.is_nan());
}
