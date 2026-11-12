//! Steering a fixed wing on the onboard navigator's solution: heading,
//! altitude, and airspeed demands instead of velocities.
//!
//! A fixed wing flown on the onboard navigator has no position or velocity
//! of its own flight controller to fly a velocity with: without satellite
//! navigation its estimator reports both invalid, while its attitude,
//! height, and airspeed stay valid. So the engine turns guidance into what
//! the flight controller can hold with its own sensors: a true heading, an
//! altitude above the ellipsoid, and an indicated airspeed. The heading is
//! the course guidance asks for over the ground, corrected for the wind
//! the navigator's ground velocity shows against the air velocity.
//!
//! When the navigator has no solution fit to fly, the engine never goes
//! silent: silence is the flight controller's command-loss descent. It
//! holds the heading and altitude it last commanded, and says why.

use navigate_contract::{GuidanceSetpoint, MonotonicNanos, NavigationSolution};
use pilotage_protocol::{AltitudeReference, ControlIntent, HeadingAltitudeIntent};

use crate::config::OwnshipSource;

mod heading_bias;

use super::{MissionEngine, MissionEvent, MissionState, StartWithheldReason};
use pilotage_mission_core::FlightAction as CoreFlightAction;

/// How long the wind estimate takes to follow a change, seconds: long
/// against a turn, in which the air-velocity model is least exact, short
/// against a change of wind along a route.
const WIND_TIME_CONSTANT_S: f64 = 10.0;

/// The heading rate above which a solution says nothing about the wind,
/// rad/s: in a turn the navigator's velocity lags the heading, and the
/// difference reads as a wind that turns with the aircraft. Well under a
/// standard-rate turn's 0.052 rad/s, above a straight leg's corrections.
const WIND_STEADY_RATE_RPS: f64 = 0.02;

/// How long the heading must have been steady before a solution says
/// anything about the wind, seconds: the navigator's velocity has caught
/// up with the end of a turn by then.
const WIND_SETTLE_S: f64 = 5.0;

/// The steering state of a fixed wing on the onboard navigator.
#[derive(Debug, Default)]
pub(super) struct Steering {
    /// The heading and altitude last commanded, radians and metres.
    last: Option<(f64, f64)>,
    /// The wind, north and east m/s, and when it was last updated.
    wind: Option<([f64; 2], MonotonicNanos)>,
    /// The heading of the last solution offered to the wind estimate, and
    /// when, so a turn can be told from a straight.
    wind_heading: Option<(f64, MonotonicNanos)>,
    /// Since when the heading has been steady.
    wind_steady_since: Option<MonotonicNanos>,
    /// Why the engine holds heading and altitude, while it does.
    holding: Option<StartWithheldReason>,
    /// The indicated airspeed last commanded on the velocity path, m/s,
    /// which the wind estimate takes as the air speed it is flown at.
    indicated_mps: Option<f64>,
    /// The flight controller's heading bias.
    heading_bias: heading_bias::HeadingBias,
}

impl MissionEngine {
    /// Whether the engine steers by heading and altitude: a fixed wing
    /// flown on the onboard navigator.
    pub(super) fn steers_by_heading(&self) -> bool {
        !self.config.can_stop()
            && matches!(
                self.config.ownship_source,
                OwnshipSource::OnboardNavigation { .. }
            )
    }

    /// The heading and altitude demand for a guidance setpoint toward a
    /// point at `altitude_m` above the ellipsoid. The setpoint's horizontal
    /// direction is the course over the ground; with none, the last
    /// commanded heading holds.
    pub(super) fn steer(
        &mut self,
        solution: &NavigationSolution,
        setpoint: &GuidanceSetpoint,
        altitude_m: f64,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        self.end_hold();
        self.update_wind(solution, now);
        self.update_heading_bias(now);
        self.report_heading_bias(events);
        let course = match setpoint {
            GuidanceSetpoint::Velocity { velocity }
                if velocity.north_mps.hypot(velocity.east_mps) > 0.0 =>
            {
                Some(velocity.east_mps.atan2(velocity.north_mps))
            }
            GuidanceSetpoint::DeviationTracking {
                lateral_m,
                course_rad,
                ..
            } => Some(course_rad + deviation_intercept(*lateral_m)),
            _ => None,
        };
        let heading = match course {
            Some(course) => self.unbiased_heading(
                self.wind_corrected_heading(course, solution.position.altitude_m),
            ),
            None => self.current_heading()?,
        };
        self.record_heading_command(heading, now);
        Some(self.heading_altitude(heading, altitude_m))
    }

    /// The heading and altitude demand that climbs, or levels, at
    /// `altitude_m` on the present heading.
    pub(super) fn steer_level(&mut self, altitude_m: f64) -> Option<ControlIntent> {
        self.end_hold();
        let heading = self.current_heading()?;
        Some(self.heading_altitude(heading, altitude_m))
    }

    /// The hold of the heading and altitude last commanded while no
    /// solution is fit to fly, announced once for each reason.
    /// Before any command the present heading and the last known altitude
    /// are held; with no heading known nothing can be held, and the event
    /// still says why.
    pub(super) fn hold_heading(
        &mut self,
        fallback_altitude_m: f64,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        self.announce_hold(events);
        self.repeat_heading(fallback_altitude_m)
    }

    /// The heading and altitude intent of a tick in `state`: guidance
    /// steered by heading while a solution is fit, the climb or the level
    /// flight of the arm request on the present heading, and the hold
    /// while no solution is fit in flight.
    pub(super) fn heading_intent_for_state(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let cruise_altitude_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
        let in_flight = matches!(
            state,
            MissionState::Climb | MissionState::Enroute | MissionState::Complete
        );
        if in_flight && solution.is_none() {
            // In a holding pattern the racetrack goes on by time; anywhere
            // else the heading last commanded holds.
            if state == MissionState::Complete
                && let Some(intent) = self.timed_holding_intent(now, events)
            {
                return Some(intent);
            }
            return self.hold_heading(cruise_altitude_m, events);
        }
        // Launched with no solution fit to fly: climb on the present
        // heading to the cruise altitude rather than leave the flight
        // controller without a command.
        if state == MissionState::AwaitSolution && solution.is_none() && self.launched(now) {
            return self.hold_heading(cruise_altitude_m, events);
        }
        match state {
            MissionState::Climb => {
                let Some(CoreFlightAction::Climb { target_altitude_m }) =
                    self.active_action.as_ref()
                else {
                    return None;
                };
                let target = *target_altitude_m;
                // Along the route, at the climb's target altitude; on the
                // present heading only while there is no route guidance.
                match self.follow_plan_intent(solution, now, events) {
                    Some(ControlIntent::HeadingAltitude(demand)) => {
                        Some(self.heading_altitude(f64::from(demand.heading_rad), target))
                    }
                    _ => self.steer_level(target),
                }
            }
            MissionState::Enroute => self.follow_plan_intent(solution, now, events),
            MissionState::Complete => self.end_of_plan_intent(solution, now, events),
            MissionState::Arming if self.arm_request_sent() => {
                let altitude_m = if self.config.cruise_height_m > 0.0 {
                    cruise_altitude_m
                } else {
                    solution?.position.altitude_m
                };
                self.steer_level(altitude_m)
            }
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Failed => None,
        }
    }

    /// Announces the hold of a vehicle with no solution fit to fly, once
    /// for each reason in a row.
    pub(super) fn announce_hold(&mut self, events: &mut Vec<MissionEvent>) {
        let reason = self.solution_absence();
        let changed = self
            .steering
            .holding
            .is_none_or(|held| core::mem::discriminant(&held) != core::mem::discriminant(&reason));
        self.steering.holding = Some(reason);
        if changed {
            events.push(MissionEvent::NavigationHold { reason });
        }
    }

    /// The position hold of a vehicle that can stop, in flight with no
    /// solution fit to fly: zero velocity, which its flight controller
    /// holds on its own estimate, and only while that estimate's position
    /// is valid. Otherwise nothing, as before: a zero velocity on an
    /// estimate with no position would drift.
    pub(super) fn hold_position(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if !self.fc_position_valid(now) {
            return None;
        }
        self.announce_hold(events);
        self.last_yaw_rad?;
        Some(super::tick::outputs::zero_velocity_intent())
    }

    /// A solution is fit again: the hold ends.
    fn end_hold(&mut self) {
        self.steering.holding = None;
    }

    /// The heading to fly so the track over the ground is `course`: the
    /// air velocity at the cruise airspeed plus the wind must point along
    /// the course, so the heading turns into the wind by
    /// `asin(crosswind / airspeed)`. A crosswind beyond the airspeed
    /// cannot be flown out; the heading then turns square into it.
    fn wind_corrected_heading(&self, course: f64, altitude_m: f64) -> f64 {
        let Some(([north, east], _)) = self.steering.wind else {
            return course;
        };
        let airspeed = crate::true_airspeed_for(self.commanded_cruise_mps(), altitude_m);
        if airspeed.is_nan() || airspeed <= 0.0 {
            return course;
        }
        // The wind component to the right of the course pushes the track
        // right; the heading turns left by as much.
        let crosswind = -north * course.sin() + east * course.cos();
        course - (crosswind / airspeed).clamp(-1.0, 1.0).asin()
    }

    /// Folds one solution into the wind estimate: the navigator's ground
    /// velocity less the air velocity, which is the cruise true airspeed
    /// along the solution's heading. A solution without a heading, or one
    /// taken in a turn, leaves the estimate as it is.
    pub(super) fn update_wind(&mut self, solution: &NavigationSolution, now: MonotonicNanos) {
        let Some(heading) = self.last_yaw_rad else {
            return;
        };
        let turning = self.steering.wind_heading.is_some_and(|(previous, at)| {
            let dt_s = now
                .elapsed_since(at)
                .map_or(0.0, |elapsed| elapsed.as_nanos() as f64 * 1e-9);
            let turned = (heading - previous).sin().atan2((heading - previous).cos());
            dt_s > 0.0 && (turned / dt_s).abs() > WIND_STEADY_RATE_RPS
        });
        self.steering.wind_heading = Some((heading, now));
        if turning {
            self.steering.wind_steady_since = None;
            return;
        }
        let since = *self.steering.wind_steady_since.get_or_insert(now);
        let settled = now
            .elapsed_since(since)
            .is_some_and(|steady| steady.as_nanos() as f64 * 1e-9 >= WIND_SETTLE_S);
        if !settled {
            return;
        }
        let indicated = self
            .steering
            .indicated_mps
            .unwrap_or_else(|| self.commanded_cruise_mps());
        let airspeed = crate::true_airspeed_for(indicated, solution.position.altitude_m);
        let sample = [
            solution.velocity.north_mps - airspeed * heading.cos(),
            solution.velocity.east_mps - airspeed * heading.sin(),
        ];
        if !sample.iter().all(|value| value.is_finite()) {
            return;
        }
        let wind = match self.steering.wind {
            None => sample,
            Some((wind, at)) => {
                let dt_s = now
                    .elapsed_since(at)
                    .map_or(0.0, |elapsed| elapsed.as_nanos() as f64 * 1e-9);
                let gain = (dt_s / WIND_TIME_CONSTANT_S).min(1.0);
                [
                    wind[0] + (sample[0] - wind[0]) * gain,
                    wind[1] + (sample[1] - wind[1]) * gain,
                ]
            }
        };
        self.steering.wind = Some((wind, now));
    }

    /// A fixed wing's velocity intent made good over the ground in the
    /// estimated wind. Its flight controller reads the horizontal
    /// direction as the course over the ground and the horizontal speed as
    /// the indicated airspeed. The engine means a ground velocity: the
    /// course it asks for and, as speed over the ground, the true airspeed
    /// that the asked speed gives in still air at `altitude_m`. So the
    /// direction stays the course, and the speed becomes the indicated
    /// airspeed whose air velocity plus the wind is that ground velocity,
    /// at most the horizontal ceiling. With no wind estimate, or an intent
    /// with no direction, the intent stands as it is.
    pub(super) fn airspeed_for_track(
        &mut self,
        intent: ControlIntent,
        yaw_rad: f64,
        altitude_m: f64,
    ) -> ControlIntent {
        let ControlIntent::Velocity(mut velocity) = intent else {
            return intent;
        };
        let (forward, right) = (f64::from(velocity.vx), f64::from(velocity.vy));
        let asked = forward.hypot(right);
        let Some(([north, east], _)) = self.steering.wind else {
            self.steering.indicated_mps = Some(asked);
            return intent;
        };
        if !(asked > 0.0 && asked.is_finite()) {
            return intent;
        }
        // The wind in the body frame the intent is written in.
        let (sin, cos) = yaw_rad.sin_cos();
        let wind = [north * cos + east * sin, -north * sin + east * cos];
        let ground_speed = crate::true_airspeed_for(asked, altitude_m);
        let ground = [forward / asked * ground_speed, right / asked * ground_speed];
        let true_airspeed = (ground[0] - wind[0]).hypot(ground[1] - wind[1]);
        let indicated = crate::indicated_airspeed_for(true_airspeed, altitude_m)
            .min(self.config.limits.max_horizontal_mps);
        if !indicated.is_finite() {
            return intent;
        }
        self.steering.indicated_mps = Some(indicated);
        velocity.vx = (forward / asked * indicated) as f32;
        velocity.vy = (right / asked * indicated) as f32;
        ControlIntent::Velocity(velocity)
    }

    /// The wind the navigator's ground velocity shows, north and east m/s;
    /// `None` before a first estimate.
    pub(super) fn wind_estimate(&self) -> Option<[f64; 2]> {
        self.steering.wind.map(|(wind, _)| wind)
    }

    /// The heading and altitude last commanded, again, or the present
    /// heading at `altitude_m` before any command.
    pub(super) fn repeat_heading(&mut self, altitude_m: f64) -> Option<ControlIntent> {
        let (heading, altitude_m) = match self.steering.last {
            Some(last) => last,
            None => (self.last_yaw_rad?, altitude_m),
        };
        Some(self.heading_altitude(heading, altitude_m))
    }

    /// The heading last commanded, or else the present one.
    pub(super) fn current_heading(&self) -> Option<f64> {
        self.steering
            .last
            .map(|(heading, _)| heading)
            .or(self.last_yaw_rad)
    }

    /// The intent for `heading` and `altitude_m` at the cruise airspeed,
    /// recorded as the last command.
    pub(super) fn heading_altitude(&mut self, heading: f64, altitude_m: f64) -> ControlIntent {
        let heading = heading.sin().atan2(heading.cos());
        self.steering.last = Some((heading, altitude_m));
        ControlIntent::HeadingAltitude(HeadingAltitudeIntent {
            heading_rad: heading as f32,
            altitude_m: altitude_m as f32,
            reference: AltitudeReference::Ellipsoid,
            airspeed_mps: Some(self.commanded_cruise_mps() as f32),
        })
    }
}

/// The angle, radians, a fixed wing turns toward a track it is
/// `lateral_m` right of: steeper the farther it is, never square to it.
fn deviation_intercept(lateral_m: f64) -> f64 {
    /// The distance off the track at which the intercept is half its
    /// greatest, 45 degrees, metres.
    const HALF_INTERCEPT_M: f64 = 500.0;
    -0.5 * (lateral_m / HALF_INTERCEPT_M).atan()
}
