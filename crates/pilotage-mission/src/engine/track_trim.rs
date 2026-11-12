//! The trim that takes a fixed wing's standing cross-track error out on a
//! straight track flown on velocity demands.
//!
//! Guidance corrects a cross-track error in proportion to it: the course
//! it asks for turns toward the track by the error over the turn radius.
//! Anything that turns the course flown away from the course asked for,
//! by a constant angle, then holds the aircraft off the track at the
//! error whose correction equals that angle: a turn radius times its
//! tangent, 30 m for 2 degrees on a C172 at 2000 m. A flight
//! controller's heading or course reference that is off true, an airspeed
//! the wind estimate does not know, or a sideslip all do this. The trim
//! integrates the cross-track error into a course offset, so the standing
//! error goes to zero whatever the cause.
//!
//! The causes that matter are the aircraft's, not the track's: a reference
//! off true turns every course by the same angle. So one trim serves every
//! track, and a new track starts with what the last one learned instead of
//! learning it again. It integrates only once the aircraft is established
//! on a track: some seconds after the track starts, close to it, and no
//! longer closing as fast as guidance asks. The capture of a track after a
//! turn is then guidance's own transient, and the trim does not wind up on
//! it.
//!
//! On heading demands the heading bias estimate owns this correction: it
//! reads the heading the flight controller holds against the one
//! commanded, and the wind estimate takes up what the air velocity does
//! not explain. A trim there corrects the same bias a second time while
//! the estimate settles, and the overshoot it leaves takes longer to go
//! than the error it took out. On velocity demands the flight controller
//! holds a course over the ground on its own estimate, which nothing here
//! reads, so the trim owns it there.

use navigate_contract::{
    GeodeticPosition, GuidanceSetpoint, LateralReference, MonotonicNanos, NavigationSolution,
};
use navigate_geodesy::{cross_track_m, distance_m, initial_bearing_rad};

use crate::body::GEODESY;

use super::MissionEngine;

/// The largest course offset the trim gives, radians (20°): more than a
/// biased reference and a crosswind the wind estimate misses together.
const MAX_TRIM_RAD: f64 = 0.35;

/// How long a track is flown before its trim integrates, seconds: the
/// capture of a new track after a turn is guidance's own transient.
const SETTLE_S: f64 = 10.0;

/// The cross-track error, as a share of the turn radius, beyond which the
/// trim does not integrate: the aircraft is still capturing the track. A
/// standing error is the turn radius times the tangent of the angle it
/// comes from, so this lets the trim take out angles up to 26°, past
/// [`MAX_TRIM_RAD`].
const GATE_SHARE_OF_RADIUS: f64 = 0.5;

/// The share of the speed toward the track that guidance asks for, its
/// cross-track gain times the error, above which the trim does not
/// integrate: guidance is then taking the error out by itself, and an
/// error on its way out must not wind the trim up. A standing error, which
/// something holds against guidance, has no speed toward the track.
const CLOSING_SHARE: f64 = 0.5;

/// A step longer than this, seconds, is a gap in the solutions, not a time
/// to integrate over.
const MAX_STEP_S: f64 = 1.0;

/// Two tracks are the same when their ends are this close, metres, and
/// their courses this close, radians.
const SAME_END_M: f64 = 1.0;
const SAME_COURSE_RAD: f64 = 0.01;

/// One straight track: where it ends and its course there.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TrackKey {
    to: GeodeticPosition,
    course_rad: f64,
}

impl TrackKey {
    fn same(&self, other: &Self) -> bool {
        let turned = (self.course_rad - other.course_rad).sin().abs();
        distance_m(&self.to, &other.to, GEODESY) <= SAME_END_M && turned <= SAME_COURSE_RAD
    }
}

/// The course trim, and the track it last stepped on.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TrackTrim {
    /// The course offset, radians, positive turning the course right.
    offset_rad: f64,
    /// The track flown, since when, and when the trim last stepped.
    track: Option<(TrackKey, MonotonicNanos, MonotonicNanos)>,
}

impl MissionEngine {
    /// `setpoint` turned by the trim of the straight track that
    /// `reference` and `to` make, after integrating the cross-track error
    /// at `solution` and `now`. A setpoint off a track, a setpoint steered
    /// by heading, and any setpoint of a vehicle that can stop are
    /// unchanged.
    pub(super) fn trimmed_setpoint(
        &mut self,
        setpoint: GuidanceSetpoint,
        reference: &LateralReference,
        to: &GeodeticPosition,
        solution: &NavigationSolution,
        now: MonotonicNanos,
    ) -> GuidanceSetpoint {
        let position = &solution.position;
        let LateralReference::Track { from } = reference else {
            return setpoint;
        };
        if self.config.can_stop() || self.steers_by_heading() {
            return setpoint;
        }
        let key = TrackKey {
            to: *to,
            course_rad: initial_bearing_rad(from, to),
        };
        let Ok(cross_m) = cross_track_m(position, from, to, GEODESY) else {
            return setpoint;
        };
        let (sin, cos) = key.course_rad.sin_cos();
        let velocity = &solution.velocity;
        let across_mps = -velocity.north_mps * sin + velocity.east_mps * cos;
        // Right of the track, closing means moving left.
        let closing_mps = -across_mps * cross_m.signum();
        let asked_mps = self.guidance.cross_track_gain_per_s * cross_m.abs();
        let established = cross_m.is_finite()
            && cross_m.abs() <= self.trim_gate_m()
            && closing_mps <= CLOSING_SHARE * asked_mps;
        let gain = self.trim_gain_per_m_s();
        let offset = self.track_trim.step(key, cross_m, gain, established, now);
        rotated(setpoint, offset)
    }

    /// The integral gain of the trim, radians per metre-second: the gain
    /// that, with guidance's cross-track gain `k = V / R`, damps the
    /// track's error with a damping ratio of 0.7, `k² / (2 V)`.
    fn trim_gain_per_m_s(&self) -> f64 {
        let gain = self.guidance.cross_track_gain_per_s;
        let speed = crate::true_airspeed_for(
            self.commanded_cruise_mps(),
            self.config.anchor.altitude_m + self.config.cruise_height_m,
        );
        if gain > 0.0 && speed > 0.0 {
            gain * gain / (2.0 * speed)
        } else {
            0.0
        }
    }

    /// The cross-track error beyond which the trim does not integrate,
    /// metres.
    fn trim_gate_m(&self) -> f64 {
        let gain = self.guidance.cross_track_gain_per_s;
        let speed = crate::true_airspeed_for(
            self.commanded_cruise_mps(),
            self.config.anchor.altitude_m + self.config.cruise_height_m,
        );
        if gain > 0.0 {
            GATE_SHARE_OF_RADIUS * speed / gain
        } else {
            0.0
        }
    }
}

impl TrackTrim {
    /// The course offset for the track `key` at `now`, radians, after
    /// integrating `cross_m` with `gain` while the aircraft is
    /// `established` on the track.
    fn step(
        &mut self,
        key: TrackKey,
        cross_m: f64,
        gain: f64,
        established: bool,
        now: MonotonicNanos,
    ) -> f64 {
        let (since, last) = match self.track {
            Some((track, since, last)) if track.same(&key) => (since, last),
            _ => (now, now),
        };
        let dt_s = seconds_between(last, now);
        let settled = seconds_between(since, now) >= SETTLE_S;
        if settled && established && dt_s > 0.0 && dt_s <= MAX_STEP_S {
            // Right of the track, the course turns left.
            self.offset_rad =
                (self.offset_rad - gain * cross_m * dt_s).clamp(-MAX_TRIM_RAD, MAX_TRIM_RAD);
        }
        self.track = Some((key, since, now));
        self.offset_rad
    }
}

/// `setpoint` with its horizontal direction turned right by `offset_rad`.
fn rotated(setpoint: GuidanceSetpoint, offset_rad: f64) -> GuidanceSetpoint {
    if offset_rad == 0.0 {
        return setpoint;
    }
    match setpoint {
        GuidanceSetpoint::Velocity { mut velocity } => {
            let (sin, cos) = offset_rad.sin_cos();
            let (north, east) = (velocity.north_mps, velocity.east_mps);
            velocity.north_mps = north * cos - east * sin;
            velocity.east_mps = north * sin + east * cos;
            GuidanceSetpoint::Velocity { velocity }
        }
        GuidanceSetpoint::DeviationTracking {
            lateral_m,
            vertical_m,
            course_rad,
        } => GuidanceSetpoint::DeviationTracking {
            lateral_m,
            vertical_m,
            course_rad: course_rad + offset_rad,
        },
        other => other,
    }
}

fn seconds_between(earlier: MonotonicNanos, later: MonotonicNanos) -> f64 {
    later
        .elapsed_since(earlier)
        .map_or(0.0, |elapsed| elapsed.as_nanos() as f64 * 1e-9)
}

#[cfg(test)]
mod tests;
