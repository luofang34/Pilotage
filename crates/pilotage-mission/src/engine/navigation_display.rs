//! What a navigation display shows of the mission (ADR-0031): the plan
//! with the progress of each waypoint, the guidance of the leg flown now,
//! and the holding pattern after the plan.
//!
//! This is display context: never an input to control validation. A part
//! with nothing to show is absent, never zero: no leg guidance without a
//! solution or while nothing is flown, and no cross-track deviation on a
//! leg without a track.

use navigate_contract::{GeodeticPosition, NavigationSolution, Waypoint};
use navigate_geodesy::{distance_m, initial_bearing_rad};

use crate::body::GEODESY;
use crate::holding_pattern::HoldingPatternStatus;

use super::{
    FlightPhase, LateralStatus, MissionEngine, MissionState, NavGuidance, SteeringSource,
    VerticalStatus,
};

/// The groundspeed below which no time to go is given, m/s.
const MIN_TIME_TO_GO_SPEED_MPS: f64 = 1.0;

/// What a navigation display shows of the mission.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationDisplay {
    /// The plan's waypoints in fly order, each with its progress.
    pub waypoints: Vec<DisplayWaypoint>,
    /// The index in [`Self::waypoints`] of the waypoint the active leg
    /// flies to; `None` once the plan is complete.
    pub active_waypoint: Option<usize>,
    /// The plan's last waypoint is done.
    pub plan_complete: bool,
    /// The guidance of the leg flown now; `None` while nothing is flown or
    /// without a solution.
    pub leg: Option<LegDisplay>,
    /// The holding pattern flown after the plan; `None` before it.
    pub holding_pattern: Option<HoldingPatternStatus>,
    /// The dead reckoning the plan is flown on, with its uncertainty;
    /// `None` while the solution flown is within its limits. A display
    /// shows it beside the guidance, which it does not remove: the
    /// vehicle flies that guidance.
    pub dead_reckoning: Option<super::DeadReckoning>,
    /// The flight phase of the active leg, which sets the scale of the
    /// lateral deviation.
    pub phase: FlightPhase,
    /// Effective Required Navigation Performance of the active leg, metres.
    pub rnp_m: f64,
    /// The position source the plan is steered on and the engine's
    /// judgment of it, present with or without a leg: a display names its
    /// source from this.
    pub steering: SteeringSource,
    /// The vertical mode and the altitude it flies to, so a display
    /// annunciates a climb, a descent, or a held altitude from what the
    /// engine says.
    pub vertical: VerticalStatus,
    /// The lateral mode flown, so a display annunciates a heading hold or
    /// a racetrack flown by time, and a racetrack resting on an old wind,
    /// from what the engine says.
    pub lateral: LateralStatus,
}

/// One waypoint of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayWaypoint {
    /// The waypoint.
    pub fix: DisplayFix,
    /// Where the plan is relative to it.
    pub progress: WaypointProgress,
}

/// A named position.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayFix {
    /// The identifier.
    pub ident: String,
    /// The position: radians, and metres on the WGS84 ellipsoid.
    pub position: GeodeticPosition,
}

/// Where the plan is relative to one waypoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaypointProgress {
    /// Sequenced: its leg is flown.
    Flown,
    /// The active leg flies to it.
    Active,
    /// Still to come.
    Ahead,
}

/// Whether the fix lies ahead of or behind the vehicle along the desired
/// track, as an HSI's to-from indicator shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToFrom {
    /// The fix is ahead.
    To,
    /// The fix is behind.
    From,
}

/// The guidance of the leg flown now.
#[derive(Debug, Clone, PartialEq)]
pub struct LegDisplay {
    /// The fix the leg runs from; `None` on a direct-to leg, which runs
    /// from the present position.
    pub from: Option<DisplayFix>,
    /// The fix the leg runs to. In a holding pattern, the holding fix.
    pub to: DisplayFix,
    /// The desired track, radians clockwise from true north in `[0, 2π)`.
    /// In a holding pattern, the inbound course.
    pub desired_track_rad: f64,
    /// The cross-track deviation, metres, positive right of the desired
    /// track; `None` on a leg with no track.
    pub cross_track_m: Option<f64>,
    /// The deviation from the leg's altitude, metres, positive above;
    /// `None` with no altitude to deviate from.
    pub vertical_deviation_m: Option<f64>,
    /// The distance to [`Self::to`] over the ellipsoid, metres.
    pub distance_to_go_m: f64,
    /// The distance to go at the present groundspeed, seconds; `None`
    /// when the vehicle is too slow for one.
    pub time_to_go_s: Option<f64>,
    /// Whether [`Self::to`] is ahead along the desired track.
    pub to_from: ToFrom,
}

impl MissionEngine {
    /// What a navigation display shows of the mission now, from the state
    /// the latest [`MissionEngine::tick`] left.
    #[must_use]
    pub fn navigation_display(&self) -> NavigationDisplay {
        let complete = self.route.remaining_waypoints().is_empty();
        let active = self.route.active_index();
        let waypoints = self
            .route
            .waypoints()
            .iter()
            .enumerate()
            .map(|(index, waypoint)| DisplayWaypoint {
                fix: display_fix(waypoint),
                progress: if complete || index < active {
                    WaypointProgress::Flown
                } else if index == active {
                    WaypointProgress::Active
                } else {
                    WaypointProgress::Ahead
                },
            })
            .collect();
        NavigationDisplay {
            waypoints,
            active_waypoint: (!complete).then_some(active),
            plan_complete: self.plan_complete,
            leg: self.leg_display(),
            holding_pattern: self.holding_pattern(),
            dead_reckoning: self.dead_reckoning(),
            phase: self.flight_phase(),
            rnp_m: self.rnp_m(),
            steering: self.steering_source(),
            vertical: self.vertical_status(),
            lateral: self.lateral_status(),
        }
    }

    /// The guidance of the leg flown now: the active leg on the plan, the
    /// inbound course to the fix in the holding pattern.
    fn leg_display(&self) -> Option<LegDisplay> {
        let solution = self.last_solution.as_ref()?;
        match self.state() {
            MissionState::Climb | MissionState::Enroute => {
                let guidance = self.nav_guidance()?;
                let leg = self.route.active_leg()?;
                Some(plan_leg(
                    solution,
                    &guidance,
                    leg.from.map(display_fix),
                    display_fix(leg.to),
                ))
            }
            MissionState::AfterPlan => self.holding_pattern_leg(solution),
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Complete
            | MissionState::Failed => None,
        }
    }
}

/// The display of a plan leg from Navigate's guidance of it.
fn plan_leg(
    solution: &NavigationSolution,
    guidance: &NavGuidance,
    from: Option<DisplayFix>,
    to: DisplayFix,
) -> LegDisplay {
    let to_from = to_from(
        &solution.position,
        &to.position,
        guidance.course_rad,
        guidance.distance_to_waypoint_m,
    );
    LegDisplay {
        from,
        desired_track_rad: guidance.course_rad,
        cross_track_m: guidance.lateral_deviation_m,
        vertical_deviation_m: guidance.vertical_deviation_m,
        distance_to_go_m: guidance.distance_to_waypoint_m,
        time_to_go_s: time_to_go_s(solution, guidance.distance_to_waypoint_m),
        to_from,
        to,
    }
}

/// The holding fix's display leg for a vehicle at `solution`: inbound on
/// the pattern's inbound course, as a pilot sets the course on an HSI in a
/// hold.
pub(super) fn holding_leg(
    solution: &NavigationSolution,
    status: &HoldingPatternStatus,
    cross_track_m: f64,
) -> LegDisplay {
    let pattern = &status.pattern;
    let distance_to_go_m = distance_m(&solution.position, &pattern.fix, GEODESY);
    LegDisplay {
        from: None,
        to: DisplayFix {
            ident: pattern.fix_ident.clone(),
            position: pattern.fix,
        },
        desired_track_rad: pattern.inbound_course_rad,
        cross_track_m: Some(cross_track_m),
        vertical_deviation_m: Some(solution.position.altitude_m - pattern.fix.altitude_m),
        distance_to_go_m,
        time_to_go_s: time_to_go_s(solution, distance_to_go_m),
        to_from: to_from(
            &solution.position,
            &pattern.fix,
            pattern.inbound_course_rad,
            distance_to_go_m,
        ),
    }
}

fn display_fix(waypoint: &Waypoint) -> DisplayFix {
    DisplayFix {
        ident: waypoint.ident.as_str().to_owned(),
        position: waypoint.position,
    }
}

/// To when the bearing to `fix` is within 90° of `track_rad`. A vehicle
/// over the fix counts as to.
pub(super) fn to_from(
    position: &GeodeticPosition,
    fix: &GeodeticPosition,
    track_rad: f64,
    distance_m: f64,
) -> ToFrom {
    if distance_m < 1.0 || (initial_bearing_rad(position, fix) - track_rad).cos() >= 0.0 {
        ToFrom::To
    } else {
        ToFrom::From
    }
}

fn time_to_go_s(solution: &NavigationSolution, distance_m: f64) -> Option<f64> {
    let velocity = &solution.velocity;
    let speed = velocity.north_mps.hypot(velocity.east_mps);
    (speed.is_finite() && speed >= MIN_TIME_TO_GO_SPEED_MPS).then(|| distance_m / speed)
}
