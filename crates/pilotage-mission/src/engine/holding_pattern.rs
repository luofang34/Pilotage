//! The holding pattern of a vehicle that cannot stop, once its plan is
//! complete: the pattern at the last waypoint, and each step of its flight
//! as a target for the plan's own guidance.

use core::f64::consts::TAU;

use navigate_contract::{
    AltitudeConstraint, LateralReference, MonotonicNanos, NavigationSolution, Waypoint,
};

use crate::holding_pattern::{
    HoldingPattern, HoldingPatternStatus, PatternFlight, PatternFrame, Steer,
    holding_speed_limit_mps, holding_turn_radius_m, standard_leg_time_s,
};

use super::{MissionEngine, MissionEvent};

/// The groundspeed below which the ground velocity has no usable course,
/// m/s.
const MIN_COURSE_SPEED_MPS: f64 = 1.0;

/// The pattern flown, its frame, and the flight in it.
#[derive(Debug, Clone)]
pub(super) struct ActivePattern {
    pattern: HoldingPattern,
    frame: PatternFrame,
    flight: PatternFlight,
    /// The entry count last announced.
    announced_entries: u32,
}

impl MissionEngine {
    /// The holding pattern the vehicle flies, once its plan is complete
    /// and it has no further instruction; `None` before then and for a
    /// vehicle that can stop.
    #[must_use]
    pub fn holding_pattern(&self) -> Option<HoldingPatternStatus> {
        let active = self.holding_pattern.as_ref()?;
        Some(HoldingPatternStatus {
            pattern: active.pattern.clone(),
            entry: active.flight.entry(),
            leg: active.flight.leg(),
            circuits: active.flight.circuits(),
            outbound_time_s: active.flight.outbound_time_s(),
        })
    }

    /// The greatest indicated airspeed of the pattern flown, m/s.
    pub(super) fn holding_speed_limit_mps(&self) -> Option<f64> {
        let active = self.holding_pattern.as_ref()?;
        Some(holding_speed_limit_mps(active.pattern.fix.altitude_m))
    }

    /// The guidance target of one step of the holding pattern: the lateral
    /// reference and the point to guide to, at the holding altitude. The
    /// pattern starts at the first step.
    pub(super) fn holding_pattern_target(
        &mut self,
        solution: &NavigationSolution,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<(LateralReference, Waypoint)> {
        let course = self.ground_course(solution)?;
        let now_s = seconds(now);
        if self.holding_pattern.is_none() {
            let active = self.start_holding_pattern(solution, course, now_s)?;
            events.push(MissionEvent::HoldingPatternStarted {
                pattern: active.pattern.clone(),
            });
            self.holding_pattern = Some(active);
        }
        let active = self.holding_pattern.as_mut()?;
        let position = active.frame.point(&solution.position);
        let frame_course = active.frame.course(course);
        let steer = active.flight.step(position, frame_course, now_s);
        if active.flight.entries() != active.announced_entries {
            active.announced_entries = active.flight.entries();
            if let Some(entry) = active.flight.entry() {
                events.push(MissionEvent::HoldingPatternEntered { entry });
            }
        }
        let (reference, to) = match steer {
            Steer::Track { from, to } => (LateralReference::track(active.frame.position(from)), to),
            Steer::Toward { point } => (LateralReference::PresentPosition, point),
        };
        let altitude_m = active.pattern.fix.altitude_m;
        let mut target = *self.route.last_waypoint()?;
        target.position = active.frame.position(to);
        target.altitude = Some(AltitudeConstraint::At(altitude_m));
        Some((reference, target))
    }

    /// The pattern at the last waypoint for a vehicle at `solution` on
    /// `course` at `now_s`.
    fn start_holding_pattern(
        &self,
        solution: &NavigationSolution,
        course: f64,
        now_s: f64,
    ) -> Option<ActivePattern> {
        let last = *self.route.last_waypoint()?;
        let settings = self.config.terminal_holding_pattern;
        let altitude_m = settings.altitude_m.unwrap_or(last.position.altitude_m);
        let leg_time_s = settings
            .leg_time_s
            .unwrap_or_else(|| standard_leg_time_s(altitude_m));
        let indicated_mps = self
            .commanded_cruise_mps()
            .min(holding_speed_limit_mps(altitude_m));
        let airspeed_mps = crate::true_airspeed_for(indicated_mps, altitude_m);
        let wind = self.wind_estimate();
        let wind_mps = wind.map_or(0.0, |[north, east]| north.hypot(east));
        let inbound_course_rad = self
            .route
            .final_course_rad()
            .unwrap_or_else(|| course.rem_euclid(TAU));
        // The wind along the inbound course makes the outbound leg slower
        // or faster over the ground by twice as much as the inbound leg.
        let tailwind_mps = wind.map_or(0.0, |[north, east]| {
            north * inbound_course_rad.cos() + east * inbound_course_rad.sin()
        });
        let mut fix = last.position;
        fix.altitude_m = altitude_m;
        let pattern = HoldingPattern {
            fix_ident: last.ident.as_str().to_owned(),
            fix,
            inbound_course_rad,
            turn: settings.turn,
            leg_time_s,
            turn_radius_m: holding_turn_radius_m(
                airspeed_mps,
                self.config.route_bank_deg,
                wind_mps,
            ),
            leg_length_m: (airspeed_mps + tailwind_mps) * leg_time_s,
        };
        let frame = PatternFrame::new(&pattern)?;
        let outbound_time_s = leg_time_s * (airspeed_mps + tailwind_mps)
            / (airspeed_mps - tailwind_mps).max(0.25 * airspeed_mps);
        let mut flight = PatternFlight::new(
            pattern.turn_radius_m,
            leg_time_s,
            airspeed_mps,
            self.route.capture_radius_m(),
            outbound_time_s,
        );
        let position = frame.point(&solution.position);
        // The plan sequenced the fix: a vehicle already past it, near the
        // pattern, enters at once instead of turning back for it.
        let passed =
            position[0] >= 0.0 && position[0].hypot(position[1]) <= 2.0 * pattern.turn_radius_m;
        flight.begin(position, frame.course(course), now_s, passed);
        Some(ActivePattern {
            pattern,
            frame,
            flight,
            announced_entries: 0,
        })
    }

    /// The display leg of the holding pattern for a vehicle at `solution`.
    pub(super) fn holding_pattern_leg(
        &self,
        solution: &NavigationSolution,
    ) -> Option<super::navigation_display::LegDisplay> {
        let active = self.holding_pattern.as_ref()?;
        let status = self.holding_pattern()?;
        let cross_track_m = active.frame.right_of_inbound_m(&solution.position);
        Some(super::navigation_display::holding_leg(
            solution,
            &status,
            cross_track_m,
        ))
    }

    /// The course over the ground of `solution`, radians clockwise from
    /// true north: the ground velocity's, or the heading when the vehicle
    /// is too slow for one.
    fn ground_course(&self, solution: &NavigationSolution) -> Option<f64> {
        let velocity = &solution.velocity;
        let speed = velocity.north_mps.hypot(velocity.east_mps);
        if speed.is_finite() && speed >= MIN_COURSE_SPEED_MPS {
            return Some(velocity.east_mps.atan2(velocity.north_mps));
        }
        self.last_yaw_rad
    }
}

fn seconds(now: MonotonicNanos) -> f64 {
    now.as_nanos() as f64 * 1e-9
}
