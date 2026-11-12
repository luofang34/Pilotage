//! The standard holding pattern that a vehicle that cannot stop flies at a
//! fix when it has no further instruction (ICAO Doc 8168, FAA AIM 5-3-8).
//!
//! The pattern is a racetrack defined by its fix and its inbound course.
//! The inbound leg ends at the fix. A turn of 180° in the pattern
//! direction, right by default, starts at the fix and ends abeam it. The
//! outbound leg then runs parallel to the inbound leg on the holding side.
//! A second turn of 180° ends on the inbound course. The turns are flown
//! at the standard rate of 3°/s, or at the route bank when that gives the
//! smaller bank. The inbound leg lasts one minute at or below 14 000 ft
//! and one and a half minutes above.
//!
//! The vehicle enters the pattern at the fix by one of the three standard
//! entries. Its course at the fix, relative to the inbound course, selects
//! the entry: the 70° line and the inbound course split the directions
//! into a parallel sector of 110°, a teardrop sector of 70°, and a direct
//! sector of 180°.
//!
//! All geometry is flown over the ground. The straight legs are tracks and
//! the turns are circles of a fixed radius. The radius is sized so that
//! the downwind side of a turn needs no more than the route bank, with the
//! wind that the engine knows when the pattern starts. The outbound leg
//! ends on a timer that starts abeam the fix. After each circuit, the timer
//! changes by the error of the measured inbound time, so the inbound leg
//! converges to its standard time in any steady wind.

mod flight;
mod timed;

#[cfg(test)]
mod tests;

use navigate_contract::GeodeticPosition;
use navigate_geodesy::{LocalTangentPlane, NedOffset};

pub(crate) use flight::{PatternFlight, Steer};
pub(crate) use timed::{CircuitShape, TimedCircuit};

/// The standard rate of turn, radians per second (3°/s).
pub const STANDARD_RATE_RPS: f64 = 3.0 * core::f64::consts::PI / 180.0;

/// The altitude up to which the inbound leg lasts
/// [`LOW_LEG_TIME_S`], metres: 14 000 ft.
pub const LEG_TIME_ALTITUDE_M: f64 = 4_267.2;

/// The inbound leg time at or below [`LEG_TIME_ALTITUDE_M`], seconds.
pub const LOW_LEG_TIME_S: f64 = 60.0;

/// The inbound leg time above [`LEG_TIME_ALTITUDE_M`], seconds.
pub const HIGH_LEG_TIME_S: f64 = 90.0;

/// The direction of the turns of a holding pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoldingTurn {
    /// Right turns: the standard pattern.
    #[default]
    Right,
    /// Left turns: a non-standard pattern.
    Left,
}

/// The entry into a holding pattern, as the course at the fix selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoldingEntry {
    /// Turn at the fix in the pattern direction onto the outbound leg.
    Direct,
    /// Fly the outbound course on the non-holding side for one leg time,
    /// then turn against the pattern direction back to the inbound track.
    Parallel,
    /// Fly a course 30° off the outbound course into the holding side for
    /// one leg time, then turn in the pattern direction onto the inbound
    /// track.
    Teardrop,
}

/// The part of the pattern the vehicle flies now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoldingLeg {
    /// Direct to the fix, before the entry.
    ToFix,
    /// The entry the course at the fix selected.
    Entry(HoldingEntry),
    /// The inbound leg to the fix.
    Inbound,
    /// The turn from the fix to the outbound leg.
    FixTurn,
    /// The outbound leg.
    Outbound,
    /// The turn from the outbound leg to the inbound leg.
    OutboundTurn,
}

/// The definition of one holding pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingPattern {
    /// The identifier of the holding fix.
    pub fix_ident: String,
    /// The holding fix. Its altitude is the holding altitude on the WGS84
    /// ellipsoid, metres.
    pub fix: GeodeticPosition,
    /// The inbound course to the fix, radians clockwise from true north in
    /// `[0, 2π)`.
    pub inbound_course_rad: f64,
    /// The direction of the turns.
    pub turn: HoldingTurn,
    /// The standard time of the inbound leg, seconds.
    pub leg_time_s: f64,
    /// The radius of the two turns over the ground, metres.
    pub turn_radius_m: f64,
    /// The length of the inbound leg over the ground in the wind known
    /// when the pattern started, metres: the leg time at the inbound
    /// groundspeed.
    pub leg_length_m: f64,
}

/// What a vehicle in a holding pattern flies now.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingPatternStatus {
    /// The pattern.
    pub pattern: HoldingPattern,
    /// The entry of the last arrival at the fix; `None` before it.
    pub entry: Option<HoldingEntry>,
    /// The part of the pattern flown now.
    pub leg: HoldingLeg,
    /// The circuits completed since the entry: each one ends at the fix.
    pub circuits: u32,
    /// The time the outbound leg is flown for, seconds.
    pub outbound_time_s: f64,
}

/// The standard inbound leg time at `altitude_m`, seconds.
///
/// The limit is 14 000 ft above mean sea level. The altitude here is on
/// the WGS84 ellipsoid: the geoid is within about 110 m of the ellipsoid,
/// so the leg time can change up to that distance away from the limit.
#[must_use]
pub fn standard_leg_time_s(altitude_m: f64) -> f64 {
    if altitude_m > LEG_TIME_ALTITUDE_M {
        HIGH_LEG_TIME_S
    } else {
        LOW_LEG_TIME_S
    }
}

/// The greatest indicated airspeed in a holding pattern at `altitude_m`,
/// m/s (ICAO Doc 8168): 230 kt up to 14 000 ft, 240 kt up to 20 000 ft,
/// and 265 kt above. Above 34 000 ft ICAO gives Mach 0.83, which this
/// indicated airspeed does not express; 265 kt applies there too.
#[must_use]
pub fn holding_speed_limit_mps(altitude_m: f64) -> f64 {
    const KNOT_MPS: f64 = 1_852.0 / 3_600.0;
    const FL200_M: f64 = 6_096.0;
    let knots = if altitude_m <= LEG_TIME_ALTITUDE_M {
        230.0
    } else if altitude_m <= FL200_M {
        240.0
    } else {
        265.0
    };
    knots * KNOT_MPS
}

/// The radius over the ground of the turns of a pattern flown at
/// `true_airspeed_mps` in a wind of `wind_mps`, metres.
///
/// In still air the turn is at the standard rate, or at `route_bank_deg`
/// when that gives the smaller bank. A turn over the ground needs the most
/// bank where the wind is behind: the radius is at least the one that the
/// groundspeed there turns at `route_bank_deg`.
#[must_use]
pub fn holding_turn_radius_m(true_airspeed_mps: f64, route_bank_deg: f64, wind_mps: f64) -> f64 {
    let still_air_m = crate::turn_radius_for(
        true_airspeed_mps,
        standard_rate_bank_deg(true_airspeed_mps).min(route_bank_deg),
        0.0,
    );
    let tailwind_m =
        crate::turn_radius_for(true_airspeed_mps + wind_mps.abs(), route_bank_deg, 0.0);
    still_air_m.max(tailwind_m)
}

/// The bank of a coordinated turn at the standard rate at
/// `true_airspeed_mps`, degrees.
#[must_use]
pub fn standard_rate_bank_deg(true_airspeed_mps: f64) -> f64 {
    (true_airspeed_mps * STANDARD_RATE_RPS / crate::body::SURFACE_GRAVITY_MPS2)
        .atan()
        .to_degrees()
}

/// The entry for a vehicle that reaches the fix on `course_rad`, for a
/// pattern on `inbound_course_rad` with `turn` turns. Courses are radians
/// clockwise from true north.
///
/// For right turns, the parallel sector holds the courses up to 110° to
/// the right of the outbound course, and the teardrop sector holds the
/// outbound course and the courses up to 70° to the left of it. All other
/// courses are direct. Left turns mirror the sectors. A course on a sector
/// boundary takes the entry of the smaller sector on its side.
#[must_use]
pub fn entry_for(inbound_course_rad: f64, turn: HoldingTurn, course_rad: f64) -> HoldingEntry {
    let relative = side(turn) * (course_rad - inbound_course_rad);
    // Degrees right of the outbound course, in [0, 360).
    let from_outbound = (relative.to_degrees() - 180.0).rem_euclid(360.0);
    if from_outbound > 0.0 && from_outbound <= 110.0 {
        HoldingEntry::Parallel
    } else if from_outbound > 110.0 && from_outbound < 290.0 {
        HoldingEntry::Direct
    } else {
        HoldingEntry::Teardrop
    }
}

/// `1` for right turns and `-1` for left turns: the factor that mirrors a
/// left-turn pattern onto a right-turn one.
fn side(turn: HoldingTurn) -> f64 {
    match turn {
        HoldingTurn::Right => 1.0,
        HoldingTurn::Left => -1.0,
    }
}

/// The plane frame of one pattern: the origin at the fix, `x` along the
/// inbound course, and `y` toward the holding side. A left-turn pattern is
/// mirrored, so the pattern in this frame always turns right.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PatternFrame {
    plane: LocalTangentPlane,
    inbound_course_rad: f64,
    side: f64,
    altitude_m: f64,
}

impl PatternFrame {
    /// The frame of `pattern`, or `None` for a fix the tangent plane
    /// refuses.
    pub(crate) fn new(pattern: &HoldingPattern) -> Option<Self> {
        Some(Self {
            plane: crate::body::tangent_plane(pattern.fix).ok()?,
            inbound_course_rad: pattern.inbound_course_rad,
            side: side(pattern.turn),
            altitude_m: pattern.fix.altitude_m,
        })
    }

    /// `position` in the frame, metres.
    pub(crate) fn point(&self, position: &GeodeticPosition) -> [f64; 2] {
        let ned = self.plane.to_ned(position);
        let (sin, cos) = self.inbound_course_rad.sin_cos();
        [
            ned.north_m * cos + ned.east_m * sin,
            self.side * (-ned.north_m * sin + ned.east_m * cos),
        ]
    }

    /// The distance of `position` right of the inbound course line through
    /// the fix, metres.
    pub(crate) fn right_of_inbound_m(&self, position: &GeodeticPosition) -> f64 {
        let [_, y] = self.point(position);
        self.side * y
    }

    /// A course over the ground in the frame: radians from `x` toward `y`.
    pub(crate) fn course(&self, course_rad: f64) -> f64 {
        self.side * (course_rad - self.inbound_course_rad)
    }

    /// The position of the frame point `point` at the holding altitude.
    pub(crate) fn position(&self, point: [f64; 2]) -> GeodeticPosition {
        let [x, y] = point;
        let y = self.side * y;
        let (sin, cos) = self.inbound_course_rad.sin_cos();
        let mut position =
            self.plane
                .from_ned(&NedOffset::new(x * cos - y * sin, x * sin + y * cos, 0.0));
        position.altitude_m = self.altitude_m;
        position
    }
}
