//! Guidance: a normalized motion demand from position and velocity.
//!
//! The law schedules the closing speed in proportion to the distance, so the
//! vehicle slows as it comes near a fix and does not fly through it. It
//! corrects the demand with the measured velocity. That matters because the
//! agent does not know the plant: one vehicle tracks a velocity demand, and
//! another reads the same demand as a throttle and needs a reverse demand to
//! stop.
//!
//! Far from a fix the vehicle turns its nose to it and moves forward, which
//! also serves a vehicle that cannot move sideways. Near the fix it keeps its
//! heading and corrects along both body axes, because the bearing to a point
//! one metre away swings too fast to steer by.

use crate::directive::TurnDirection;
use crate::scenario::Fix;
use crate::state::VehicleState;

/// Yaw demand per radian of heading error.
const YAW_GAIN: f64 = 1.2;
/// Time to close the remaining distance at the scheduled speed, in seconds.
const APPROACH_TIME_S: f64 = 1.5;
/// Demand per unit of speed error, as a share of the speed limit.
const SPEED_ERROR_GAIN: f64 = 1.0;
/// Distance inside which the vehicle holds its heading and corrects along
/// both body axes, in metres.
const STATION_RANGE_M: f64 = 3.0;
/// Climb demand per metre of height error.
const HEIGHT_GAIN: f64 = 0.5;
/// Largest climb or descent demand used to hold height.
const HEIGHT_HOLD_LIMIT: f64 = 0.5;
/// Heading error inside which a commanded turn side no longer applies.
const TURN_SIDE_RELEASE_RAD: f64 = 0.35;

/// A normalized motion demand, each axis in `[-1, 1]`. Pitch is forward, roll
/// is right, positive throttle climbs, positive yaw turns right.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Demand {
    /// Rightward demand.
    pub roll: f32,
    /// Forward demand.
    pub pitch: f32,
    /// Climb demand.
    pub throttle: f32,
    /// Right-yaw demand.
    pub yaw: f32,
}

/// Speeds that scale a demand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Speeds {
    /// Advertised speed at full horizontal demand, in metres per second.
    pub max_linear_mps: f64,
    /// The commanded cruise speed, in metres per second.
    pub cruise_mps: f64,
}

/// Horizontal distance from the vehicle to a fix, in metres.
pub(crate) fn distance_m(state: &VehicleState, fix: Fix) -> f64 {
    (fix.north_m - state.north_m).hypot(fix.east_m - state.east_m)
}

/// Horizontal ground speed, in metres per second.
pub(crate) fn ground_speed_mps(state: &VehicleState) -> f64 {
    state.vel_north_mps.hypot(state.vel_east_mps)
}

/// The climb demand that holds `height_m`. It is zero for a planar vehicle.
pub(crate) fn height_demand(state: &VehicleState, height_m: f64) -> f32 {
    state.height_m.map_or(0.0, |height| {
        narrow((HEIGHT_GAIN * (height_m - height)).clamp(-HEIGHT_HOLD_LIMIT, HEIGHT_HOLD_LIMIT))
    })
}

/// The demand toward `fix`, with `throttle` set by the caller.
pub(crate) fn toward(state: &VehicleState, fix: Fix, speeds: Speeds, throttle: f32) -> Demand {
    let north = fix.north_m - state.north_m;
    let east = fix.east_m - state.east_m;
    let range = north.hypot(east);
    // The scheduled velocity points at the fix. Its size is the distance over
    // the approach time, and the cruise speed bounds it.
    let scale = if range > f64::EPSILON {
        (range / APPROACH_TIME_S).min(speeds.cruise_mps) / range
    } else {
        0.0
    };
    let (forward, right) = body_demand(state, (north * scale, east * scale), speeds);
    if range <= STATION_RANGE_M {
        return Demand {
            roll: narrow(right),
            pitch: narrow(forward),
            throttle,
            yaw: 0.0,
        };
    }
    let error = wrap_pi(east.atan2(north) - state.yaw_rad);
    Demand {
        roll: 0.0,
        pitch: narrow(gate_forward(forward, error)),
        throttle,
        yaw: narrow((YAW_GAIN * error).clamp(-1.0, 1.0)),
    }
}

/// The demand that turns to `heading_rad` on the commanded side and flies it.
pub(crate) fn along_heading(
    state: &VehicleState,
    heading_rad: f64,
    turn: TurnDirection,
    speeds: Speeds,
    throttle: f32,
) -> Demand {
    let short_error = wrap_pi(heading_rad - state.yaw_rad);
    // A commanded side can mean the long way round. It applies until the nose
    // is near the heading; after that the short way and the commanded way are
    // the same.
    let far = short_error.abs() > TURN_SIDE_RELEASE_RAD;
    let error = match turn {
        TurnDirection::Left if far && short_error > 0.0 => short_error - std::f64::consts::TAU,
        TurnDirection::Right if far && short_error < 0.0 => short_error + std::f64::consts::TAU,
        _ => short_error,
    };
    let facing = short_error.cos().max(0.0).powi(2);
    let (sin, cos) = heading_rad.sin_cos();
    let want = (
        speeds.cruise_mps * facing * cos,
        speeds.cruise_mps * facing * sin,
    );
    // Both body axes take part, so a side wind or a late turn does not leave
    // the vehicle on a track beside the heading.
    let (forward, right) = body_demand(state, want, speeds);
    Demand {
        roll: narrow(right),
        pitch: narrow(forward),
        throttle,
        yaw: narrow((YAW_GAIN * error).clamp(-1.0, 1.0)),
    }
}

/// The wanted velocity, corrected with the measured velocity, in body axes and
/// as a share of full demand.
fn body_demand(state: &VehicleState, want: (f64, f64), speeds: Speeds) -> (f64, f64) {
    let limit = speeds.max_linear_mps.max(f64::EPSILON);
    let north = want.0 + SPEED_ERROR_GAIN * (want.0 - state.vel_north_mps);
    let east = want.1 + SPEED_ERROR_GAIN * (want.1 - state.vel_east_mps);
    let (sin, cos) = state.yaw_rad.sin_cos();
    let forward = (cos * north + sin * east) / limit;
    let right = (cos * east - sin * north) / limit;
    (forward.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0))
}

/// A forward demand falls off with the square of the cosine, so the vehicle
/// turns first and then goes. A reverse demand is a brake and stays whole.
fn gate_forward(forward: f64, heading_error: f64) -> f64 {
    if forward > 0.0 {
        forward * heading_error.cos().max(0.0).powi(2)
    } else {
        forward
    }
}

/// Wraps an angle into `(-pi, pi]`.
pub(crate) fn wrap_pi(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(std::f64::consts::TAU);
    if wrapped > std::f64::consts::PI {
        wrapped - std::f64::consts::TAU
    } else {
        wrapped
    }
}

/// Demands are bounded to `[-1, 1]` before this call, so the narrowing loses
/// precision only.
#[allow(clippy::cast_possible_truncation)]
fn narrow(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests;
