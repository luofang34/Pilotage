//! The angle at which a fixed wing intercepts a track.
//!
//! Guidance turns the course it asks for toward a track in proportion to
//! the cross-track error, up to the speed ceiling, so far from the track it
//! asks for a course nearly square to it. A fixed wing flown that way
//! arrives square to the track at full speed and swings past it by a turn
//! radius before it can turn along it, as it does after a fly-over turn of
//! 90° or more. The course is held within [`MAX_INTERCEPT_RAD`] of the
//! track instead, as an intercept is flown. Within a turn radius of the
//! track the proportional course is already inside that angle, and the
//! aircraft turns onto the track as the angle falls.

use core::f64::consts::FRAC_PI_4;

use navigate_contract::{GeodeticPosition, GuidanceSetpoint, LateralReference};
use navigate_geodesy::initial_bearing_rad;

use super::MissionEngine;

/// The greatest angle between the course asked for and the track, radians.
const MAX_INTERCEPT_RAD: f64 = FRAC_PI_4;

impl MissionEngine {
    /// `setpoint` with its course within [`MAX_INTERCEPT_RAD`] of the
    /// straight track that `reference` and `to` make. A setpoint off a
    /// track, and any setpoint of a vehicle that can stop, is unchanged.
    pub(super) fn intercept_setpoint(
        &self,
        setpoint: GuidanceSetpoint,
        reference: &LateralReference,
        to: &GeodeticPosition,
    ) -> GuidanceSetpoint {
        let LateralReference::Track { from } = reference else {
            return setpoint;
        };
        if self.config.can_stop() {
            return setpoint;
        }
        within_intercept(setpoint, initial_bearing_rad(from, to))
    }
}

/// `setpoint` with its course within [`MAX_INTERCEPT_RAD`] of `track_rad`,
/// at its own horizontal speed.
fn within_intercept(setpoint: GuidanceSetpoint, track_rad: f64) -> GuidanceSetpoint {
    match setpoint {
        GuidanceSetpoint::Velocity { mut velocity } => {
            let speed = velocity.north_mps.hypot(velocity.east_mps);
            let course = velocity.east_mps.atan2(velocity.north_mps);
            let off = wrap(course - track_rad);
            if speed.is_nan() || speed <= 0.0 || off.abs() <= MAX_INTERCEPT_RAD {
                return setpoint;
            }
            let held = track_rad + MAX_INTERCEPT_RAD.copysign(off);
            velocity.north_mps = speed * held.cos();
            velocity.east_mps = speed * held.sin();
            GuidanceSetpoint::Velocity { velocity }
        }
        other => other,
    }
}

fn wrap(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}

#[cfg(test)]
mod tests;
