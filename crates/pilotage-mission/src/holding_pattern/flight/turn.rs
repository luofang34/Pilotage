//! A turn of the pattern: a circle over the ground at a fixed centre,
//! flown toward a guided point ahead on it and ended at a course.

use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// How far ahead of the vehicle's bearing from a turn centre the guided
/// point lies, radians. A vehicle at radius `r` that flies toward a point
/// this far ahead at `radius / cos(lead)` moves square to its bearing only
/// at `r = radius`, so the steady path is the circle itself.
const TURN_LEAD_RAD: f64 = FRAC_PI_4;

/// The turn still to go at which a turn ends, radians (1°).
const TURN_DONE_RAD: f64 = PI / 180.0;

/// A turn over the ground at a fixed centre, ended at a course.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Turn {
    centre: [f64; 2],
    radius_m: f64,
    clockwise: bool,
    exit_course: f64,
    /// The turn still to go at the last step, radians.
    to_go: f64,
}

impl Turn {
    /// The turn that starts at `position` on `course` and ends on
    /// `exit_course`.
    pub(super) fn new(
        position: [f64; 2],
        course: f64,
        radius_m: f64,
        clockwise: bool,
        exit_course: f64,
    ) -> Self {
        let side = if clockwise {
            course + FRAC_PI_2
        } else {
            course - FRAC_PI_2
        };
        Self {
            centre: offset(position, side, radius_m),
            radius_m,
            clockwise,
            exit_course,
            to_go: sweep(course, exit_course, clockwise),
        }
    }

    /// The bearing of `position` from the centre.
    fn bearing(&self, position: [f64; 2]) -> f64 {
        (position[1] - self.centre[1]).atan2(position[0] - self.centre[0])
    }

    /// Advances the turn to `position`; true when it is done: the turn to
    /// go is nearly nothing, or it passed through nothing since the last
    /// step.
    pub(super) fn advance(&mut self, position: [f64; 2]) -> bool {
        let tangent = if self.clockwise {
            FRAC_PI_2
        } else {
            -FRAC_PI_2
        };
        let course = self.bearing(position) + tangent;
        let to_go = sweep(course, self.exit_course, self.clockwise);
        let done = to_go < TURN_DONE_RAD || to_go > self.to_go + PI;
        self.to_go = to_go;
        done
    }

    /// How far `position` is outside the turn's circle, metres.
    pub(super) fn radial_error_m(&self, position: [f64; 2]) -> f64 {
        (position[0] - self.centre[0]).hypot(position[1] - self.centre[1]) - self.radius_m
    }

    /// The guided point of the turn for a vehicle at `position`, on a
    /// circle `inward_m` inside the turn's own.
    pub(super) fn lead(&self, position: [f64; 2], inward_m: f64) -> [f64; 2] {
        let lead = if self.clockwise {
            TURN_LEAD_RAD
        } else {
            -TURN_LEAD_RAD
        };
        offset(
            self.centre,
            self.bearing(position) + lead,
            (self.radius_m - inward_m) / TURN_LEAD_RAD.cos(),
        )
    }
}

/// The turn from `from` to `to` in the given direction, radians in
/// `[0, 2π)`.
fn sweep(from: f64, to: f64, clockwise: bool) -> f64 {
    let turn = if clockwise { to - from } else { from - to };
    turn.rem_euclid(TAU)
}

pub(super) fn offset(point: [f64; 2], course: f64, distance_m: f64) -> [f64; 2] {
    [
        point[0] + distance_m * course.cos(),
        point[1] + distance_m * course.sin(),
    ]
}
