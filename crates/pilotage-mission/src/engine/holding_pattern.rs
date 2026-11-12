//! The holding pattern of a vehicle that cannot stop, once its plan is
//! complete: the pattern at the last waypoint, and each step of its flight
//! as a target for the plan's own guidance.

use core::f64::consts::TAU;

use navigate_contract::{
    AltitudeConstraint, LateralReference, MonotonicNanos, NavigationSolution, Waypoint,
};

use crate::holding_pattern::{
    CircuitShape, HoldingLeg, HoldingPattern, HoldingPatternStatus, HoldingTurn, PatternFlight,
    PatternFrame, Steer, TimedCircuit, holding_speed_limit_mps, holding_turn_radius_m,
    standard_leg_time_s,
};
use pilotage_protocol::ControlIntent;

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
    /// The circuit flown by time while there is no solution fit to fly.
    timed: Option<TimedCircuit>,
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
        active.timed = None;
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

    /// The heading and altitude demand of the holding pattern flown by
    /// time, while there is no solution fit to fly; `None` with no pattern
    /// started or no heading to start the circuit from. The circuit takes
    /// over from the part of the pattern flown when the solution went, and
    /// the pattern is entered again from where the vehicle is when a
    /// solution comes back.
    pub(super) fn timed_holding_intent(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if !self.steers_by_heading() {
            return None;
        }
        let now_s = seconds(now);
        self.holding_pattern.as_ref()?;
        let wind = self.timed_hold_wind(now, events);
        let heading_now = self.current_heading()?;
        let indicated_mps = self.commanded_cruise_mps();
        let active = self.holding_pattern.as_mut()?;
        let altitude_m = active.pattern.fix.altitude_m;
        let timed = active.timed.get_or_insert_with(|| {
            let shape = CircuitShape {
                inbound_rad: active.pattern.inbound_course_rad,
                right_turns: active.pattern.turn == HoldingTurn::Right,
                airspeed_mps: crate::true_airspeed_for(indicated_mps, altitude_m),
                turn_radius_m: active.pattern.turn_radius_m,
                leg_time_s: active.pattern.leg_time_s,
            };
            TimedCircuit::take_over(
                shape,
                active.flight.leg(),
                active.flight.part_elapsed_s(now_s),
                active.flight.outbound_time_s(),
                heading_now,
                now_s,
                wind,
            )
        });
        let heading = timed.step(now_s, wind);
        self.announce_hold(events);
        let commanded = self.unbiased_heading(heading);
        Some(self.heading_altitude(commanded, altitude_m))
    }

    /// The heading and altitude demand of a racetrack flown by time where
    /// no pattern was flown: right turns and the standard leg time for
    /// the altitude, inbound on the heading held, begun with the turn to
    /// its outbound leg where the vehicle is. `None` with no heading to
    /// begin it from.
    pub(super) fn racetrack_intent(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let heading_now = self.current_heading()?;
        let cruise_altitude_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
        let altitude_m = self.commanded_altitude_m().unwrap_or(cruise_altitude_m);
        let now_s = seconds(now);
        let wind = self.timed_hold_wind(now, events);
        if self.modes.lateral.racetrack.is_none() {
            let indicated_mps = self
                .commanded_cruise_mps()
                .min(holding_speed_limit_mps(altitude_m));
            let airspeed_mps = crate::true_airspeed_for(indicated_mps, altitude_m);
            let leg_time_s = standard_leg_time_s(altitude_m);
            let shape = CircuitShape {
                inbound_rad: heading_now,
                right_turns: true,
                airspeed_mps,
                turn_radius_m: holding_turn_radius_m(
                    airspeed_mps,
                    self.config.route_bank_deg,
                    wind[0].hypot(wind[1]),
                ),
                leg_time_s,
            };
            self.modes.lateral.racetrack = Some(TimedCircuit::take_over(
                shape,
                HoldingLeg::FixTurn,
                0.0,
                leg_time_s,
                heading_now,
                now_s,
                wind,
            ));
            events.push(MissionEvent::TimedHoldStarted {
                inbound_course_rad: heading_now.rem_euclid(TAU),
                leg_time_s,
            });
        }
        let heading = self.modes.lateral.racetrack.as_mut()?.step(now_s, wind);
        self.announce_hold(events);
        let commanded = self.unbiased_heading(heading);
        Some(self.heading_altitude(commanded, altitude_m))
    }

    /// The wind a racetrack flown by time is corrected for, north and
    /// east m/s: the last estimate while it is no older than the
    /// configured age, and still air once it is, announced once.
    fn timed_hold_wind(&mut self, now: MonotonicNanos, events: &mut Vec<MissionEvent>) -> [f64; 2] {
        let Some((wind, at)) = self.wind_estimate_stamped() else {
            return [0.0, 0.0];
        };
        let age_ns = now.elapsed_since(at).map_or(0, |age| age.as_nanos());
        if age_ns <= self.config.timed_hold_wind_max_age.as_nanos() {
            return wind;
        }
        if !self.modes.lateral.wind_dropped {
            self.modes.lateral.wind_dropped = true;
            events.push(MissionEvent::TimedHoldWindDropped {
                age_s: age_ns as f64 * 1e-9,
            });
        }
        [0.0, 0.0]
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
            timed: None,
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
