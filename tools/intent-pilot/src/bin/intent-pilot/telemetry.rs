//! Wire telemetry into the state types of the agent core.
//!
//! The control state and truth come from separate telemetry groups. The
//! executor flies on the operational estimate. The verifier reads truth.

use pilotage_agent::{TruthState, VehicleState, yaw_of_quaternion};
use pilotage_protocol::wire;

/// `FcState.arm_state` value that the adapters publish for an armed vehicle.
const ARM_STATE_ARMED: u32 = 2;

/// Reads the control state. The stamped avionics estimate is preferred. The
/// planar pose serves a vehicle that has no avionics group.
pub(crate) fn control_state(sample: &wire::TelemetrySample) -> Option<VehicleState> {
    let armed = sample
        .fc_state
        .as_ref()
        .map(|fc| fc.arm_state == ARM_STATE_ARMED);
    if let Some(avionics) = sample.avionics.as_ref()
        && avionics.kinematics_stamp.is_some()
    {
        // A missing attitude group leaves yaw unknown. Guidance cannot steer
        // without it, and a zero would read as due north.
        avionics.attitude_stamp.as_ref()?;
        return Some(VehicleState {
            north_m: f64::from(avionics.pos_n_m),
            east_m: f64::from(avionics.pos_e_m),
            vel_north_mps: f64::from(avionics.vel_n_mps),
            vel_east_mps: f64::from(avionics.vel_e_mps),
            height_m: Some(-f64::from(avionics.pos_d_m)),
            climb_mps: -f64::from(avionics.vel_d_mps),
            yaw_rad: quat_yaw(
                avionics.quat_w,
                avionics.quat_x,
                avionics.quat_y,
                avionics.quat_z,
            ),
            armed,
        });
    }
    let pose = sample.pose.as_ref()?;
    // The planar vehicle reports one speed, along its heading.
    let speed = sample
        .velocity
        .as_ref()
        .map_or(0.0, |velocity| f64::from(velocity.linear_x_mps));
    let (sin, cos) = f64::from(pose.heading_rad).sin_cos();
    Some(VehicleState {
        north_m: f64::from(pose.x_m),
        east_m: f64::from(pose.y_m),
        vel_north_mps: speed * cos,
        vel_east_mps: speed * sin,
        height_m: None,
        climb_mps: 0.0,
        yaw_rad: f64::from(pose.heading_rad),
        armed,
    })
}

/// Reads truth. `planar_pose_is_truth` is the operator's declaration that the
/// host runs the deterministic reference vehicle, whose planar pose is the
/// simulator's own state. The host makes the same declaration for its mission
/// principal.
pub(crate) fn truth_state(
    sample: &wire::TelemetrySample,
    planar_pose_is_truth: bool,
) -> Option<TruthState> {
    if let Some(truth) = sample.sim_truth.as_deref() {
        // Truth without a stamp has no provenance and is not used.
        truth.stamp.as_ref()?;
        return Some(TruthState {
            north_m: f64::from(truth.pos_n_m),
            east_m: f64::from(truth.pos_e_m),
            height_m: Some(-f64::from(truth.pos_d_m)),
            yaw_rad: Some(quat_yaw(
                truth.quat_w,
                truth.quat_x,
                truth.quat_y,
                truth.quat_z,
            )),
        });
    }
    if !planar_pose_is_truth {
        return None;
    }
    let pose = sample.pose.as_ref()?;
    Some(TruthState {
        north_m: f64::from(pose.x_m),
        east_m: f64::from(pose.y_m),
        height_m: None,
        yaw_rad: Some(f64::from(pose.heading_rad)),
    })
}

/// Yaw of a wire quaternion, in radians from north toward east.
fn quat_yaw(w: f32, x: f32, y: f32, z: f32) -> f64 {
    yaw_of_quaternion(f64::from(w), f64::from(x), f64::from(y), f64::from(z))
}
