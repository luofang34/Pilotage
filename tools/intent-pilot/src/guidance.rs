//! Waypoint guidance: a normalized motion demand from position and velocity.
//!
//! The law schedules the closing speed in proportion to the distance, so
//! the vehicle slows as it comes near the waypoint and does not fly through
//! it. It corrects the demand with the measured velocity. That matters
//! because the pilot does not know the plant: one vehicle tracks a velocity
//! demand, and another reads the same demand as a throttle and needs a
//! reverse demand to stop.
//!
//! Far from the waypoint the vehicle turns its nose to it and moves forward,
//! which also serves a vehicle that cannot move sideways. Near the waypoint
//! it keeps its heading and corrects along both body axes, because the
//! bearing to a point one metre away swings too fast to steer by.

use pilotage_client_session::MotionDemand;

use crate::scenario::Waypoint;
use crate::vehicle_state::VehicleState;

/// Yaw demand per radian of bearing error.
const YAW_GAIN: f64 = 1.2;
/// Time to close the remaining distance at the scheduled speed, in
/// seconds. The scheduled closing speed is the distance divided by it.
const APPROACH_TIME_S: f64 = 1.5;
/// Share of the advertised speed limit used as the cruise speed.
const CRUISE_SHARE: f64 = 0.8;
/// Demand per unit of speed error, as a share of the speed limit.
const SPEED_ERROR_GAIN: f64 = 1.0;
/// Distance inside which the vehicle holds its heading and corrects along
/// both body axes, in metres.
const STATION_RANGE_M: f64 = 3.0;
/// Climb demand per metre of height error.
const HEIGHT_GAIN: f64 = 0.5;
/// Largest climb or descent demand used to hold height.
const HEIGHT_HOLD_LIMIT: f64 = 0.5;

/// A motionless demand.
pub(crate) const STOP: MotionDemand = MotionDemand {
    roll: 0.0,
    pitch: 0.0,
    throttle: 0.0,
    yaw: 0.0,
};

/// Horizontal distance from the vehicle to a waypoint, in metres.
pub(crate) fn distance_m(state: &VehicleState, waypoint: Waypoint) -> f64 {
    (waypoint.north_m - state.north_m).hypot(waypoint.east_m - state.east_m)
}

/// Horizontal ground speed, in metres per second.
pub(crate) fn ground_speed_mps(state: &VehicleState) -> f64 {
    state.vel_north_mps.hypot(state.vel_east_mps)
}

/// The demand that holds `height_m` with no horizontal correction.
pub(crate) fn hold_height(state: &VehicleState, height_m: f64) -> MotionDemand {
    MotionDemand {
        throttle: height_demand(state, height_m),
        ..STOP
    }
}

/// The horizontal demand toward `waypoint`, with `throttle` set by the
/// caller. `max_linear_mps` is the advertised speed at full demand.
pub(crate) fn toward(
    state: &VehicleState,
    waypoint: Waypoint,
    max_linear_mps: f64,
    throttle: f32,
) -> MotionDemand {
    let north = waypoint.north_m - state.north_m;
    let east = waypoint.east_m - state.east_m;
    let range = north.hypot(east);
    let limit = max_linear_mps.max(f64::EPSILON);
    let cruise = CRUISE_SHARE * limit;
    // The scheduled velocity points at the waypoint. Its size is the
    // distance over the approach time, and the cruise speed bounds it.
    let scale = if range > f64::EPSILON {
        (range / APPROACH_TIME_S).min(cruise) / range
    } else {
        0.0
    };
    let (want_north, want_east) = (north * scale, east * scale);
    let demand_north = want_north + SPEED_ERROR_GAIN * (want_north - state.vel_north_mps);
    let demand_east = want_east + SPEED_ERROR_GAIN * (want_east - state.vel_east_mps);
    let (sin, cos) = state.yaw_rad.sin_cos();
    let forward = (cos * demand_north + sin * demand_east) / limit;
    let right = (cos * demand_east - sin * demand_north) / limit;

    if range <= STATION_RANGE_M {
        return MotionDemand {
            roll: narrow(right.clamp(-1.0, 1.0)),
            pitch: narrow(forward.clamp(-1.0, 1.0)),
            throttle,
            yaw: 0.0,
        };
    }
    let bearing_error = wrap_pi(east.atan2(north) - state.yaw_rad);
    // A forward demand falls off with the square of the cosine, so the
    // vehicle turns first and then goes. A reverse demand is a brake and
    // is kept whole at every heading.
    let facing = bearing_error.cos().max(0.0).powi(2);
    let pitch = if forward > 0.0 {
        forward * facing
    } else {
        forward
    };
    MotionDemand {
        roll: 0.0,
        pitch: narrow(pitch.clamp(-1.0, 1.0)),
        throttle,
        yaw: narrow((YAW_GAIN * bearing_error).clamp(-1.0, 1.0)),
    }
}

/// The climb demand that holds `height_m`. It is zero for a planar vehicle.
pub(crate) fn height_demand(state: &VehicleState, height_m: f64) -> f32 {
    state.height_m.map_or(0.0, |height| {
        narrow((HEIGHT_GAIN * (height_m - height)).clamp(-HEIGHT_HOLD_LIMIT, HEIGHT_HOLD_LIMIT))
    })
}

/// Wraps an angle into `(-pi, pi]`.
fn wrap_pi(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(std::f64::consts::TAU);
    if wrapped > std::f64::consts::PI {
        wrapped - std::f64::consts::TAU
    } else {
        wrapped
    }
}

/// Demands are bounded to `[-1, 1]` before this call, so the narrowing
/// loses precision only.
#[allow(clippy::cast_possible_truncation)]
fn narrow(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests;
