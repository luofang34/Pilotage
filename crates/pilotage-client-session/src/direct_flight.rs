//! Normalized stick demand onto the direct-flight intents (`AttitudeThrust`
//! for angle flight, `BodyRate` for acro flight), scaled by the envelope
//! the direct scope advertises.
//!
//! The refusal rules are the velocity builder's: no advertisement of the
//! family, no intent. Signs follow the aerospace body frame (FRD): a
//! positive pitch angle or pitch rate raises the nose, so a FORWARD demand
//! is a negative pitch. Thrust is normalized collective in `[0, 1]`: a
//! centered throttle stick (0) is half thrust, full down (-1) is zero.

use core::f32::consts::{PI, TAU};

use pilotage_protocol::wire;

use crate::MotionDemand;

/// The normalized collective a throttle demand in `[-1, 1]` asks for.
fn thrust(throttle: f32) -> f32 {
    ((throttle + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Builds the angle-flight intent: roll and pitch demands command TILT
/// angles up to the advertised `max_angular` (rad), and `heading_rad` is
/// the client-held heading setpoint (see [`integrate_heading`]). The
/// quaternion is the ZYX Euler composition in local NED.
///
/// Returns `None` without an `AttitudeThrust` advertisement that admits
/// local NED, the only frame a world-referenced heading states faithfully.
#[must_use]
pub fn attitude_thrust_intent(
    demand: MotionDemand,
    heading_rad: f32,
    capability: Option<&wire::IntentCapability>,
) -> Option<wire::ControlIntent> {
    let capability = capability?;
    if capability.family != wire::IntentFamily::AttitudeThrust as i32 {
        return None;
    }
    let local_ned = wire::ReferenceFrame::LocalNed as i32;
    if !capability.frames.contains(&local_ned) {
        return None;
    }
    let roll = demand.roll * capability.max_angular;
    let pitch = -demand.pitch * capability.max_angular;
    let (sr, cr) = (roll / 2.0).sin_cos();
    let (sp, cp) = (pitch / 2.0).sin_cos();
    let (sy, cy) = (heading_rad / 2.0).sin_cos();
    Some(wire::ControlIntent {
        family: Some(wire::control_intent::Family::AttitudeThrust(
            wire::AttitudeThrustIntent {
                frame: local_ned,
                qw: cr * cp * cy + sr * sp * sy,
                qx: sr * cp * cy - cr * sp * sy,
                qy: cr * sp * cy + sr * cp * sy,
                qz: cr * cp * sy - sr * sp * cy,
                thrust: thrust(demand.throttle),
            },
        )),
    })
}

/// Advances the angle-flight heading setpoint by one tick: the yaw demand
/// slews it at the advertised `max_yaw_rate` (rad/s) for `dt_s` seconds.
/// The result is wrapped to `(-π, π]`. The client holds the heading on the
/// direct scope, so the vehicle never reads a stick as a rate there.
#[must_use]
pub fn integrate_heading(
    heading_rad: f32,
    yaw_demand: f32,
    capability: Option<&wire::IntentCapability>,
    dt_s: f32,
) -> f32 {
    let rate = capability.map_or(0.0, |capability| capability.max_yaw_rate);
    let turned = (heading_rad + yaw_demand * rate * dt_s) % TAU;
    if turned > PI {
        turned - TAU
    } else if turned <= -PI {
        turned + TAU
    } else {
        turned
    }
}

/// Builds the acro-flight intent: roll, pitch, and yaw demands command body
/// rates up to the advertised envelope (rad/s), with normalized thrust. Yaw
/// uses `max_yaw_rate` when the scope advertises one and `max_angular`
/// otherwise.
///
/// Returns `None` without a `BodyRate` advertisement.
#[must_use]
pub fn body_rate_intent(
    demand: MotionDemand,
    capability: Option<&wire::IntentCapability>,
) -> Option<wire::ControlIntent> {
    let capability = capability?;
    if capability.family != wire::IntentFamily::BodyRate as i32 {
        return None;
    }
    let max_yaw_rate = if capability.max_yaw_rate > 0.0 {
        capability.max_yaw_rate
    } else {
        capability.max_angular
    };
    Some(wire::ControlIntent {
        family: Some(wire::control_intent::Family::BodyRate(
            wire::BodyRateIntent {
                roll_rate: demand.roll * capability.max_angular,
                pitch_rate: -demand.pitch * capability.max_angular,
                yaw_rate: demand.yaw * max_yaw_rate,
                thrust: thrust(demand.throttle),
            },
        )),
    })
}

#[cfg(test)]
mod tests;
