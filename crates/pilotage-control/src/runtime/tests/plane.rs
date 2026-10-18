#![allow(clippy::expect_used, clippy::panic)]

//! The plane scheme flies the same BODY_FRD velocity intent as every other
//! vehicle: right stick Y is forward speed, right stick X the turn, left
//! stick Y the climb, and nothing commands sideslip.

use pilotage_protocol::VehicleClass;

use super::{sample, session, with_default};
use crate::plan::{AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, LeaseAction};
use crate::sample::Mode;

fn axis(plan: &crate::plan::ControlPlan, id: u16) -> f32 {
    plan.motion
        .as_ref()
        .expect("motion frame")
        .axes()
        .iter()
        .find(|(axis, _)| *axis == id)
        .map(|(_, value)| *value)
        .expect("axis present")
}

#[test]
fn the_plane_sticks_map_to_speed_turn_and_climb() {
    let mut runtime = with_default();
    // Left X full right, left Y full up, right X full right, right Y full up.
    let plan = runtime.evaluate(
        &sample(&[1.0, -1.0, 1.0, -1.0], &[]),
        &session(Mode::Plane, true),
    );
    assert_eq!(axis(&plan, AXIS_PITCH), 1.0, "right Y up is forward speed");
    assert_eq!(axis(&plan, AXIS_YAW), 1.0, "right X right is a right turn");
    assert_eq!(axis(&plan, AXIS_THROTTLE), 1.0, "left Y up is climb");
    assert_eq!(axis(&plan, AXIS_ROLL), 0.0, "a plane commands no sideslip");
}

#[test]
fn the_plane_releases_the_gimbal_lease() {
    let mut runtime = with_default();
    let plan = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::Plane, true),
    );
    assert_eq!(plan.lease, Some(LeaseAction::Release));
}

#[test]
fn the_advertised_vehicle_class_chooses_the_scheme() {
    let table = [
        (Some(VehicleClass::Multicopter), Mode::QuadPilot),
        (Some(VehicleClass::FixedWing), Mode::Plane),
        (Some(VehicleClass::Vtol), Mode::QuadPilot),
        (Some(VehicleClass::GroundRover), Mode::Rover),
        (Some(VehicleClass::SurfaceVessel), Mode::Rover),
        (None, Mode::QuadPilot),
    ];
    for (class, mode) in table {
        assert_eq!(Mode::for_vehicle_class(class), mode, "{class:?}");
    }
}
