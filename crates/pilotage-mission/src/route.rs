//! The owned mission route: the waypoint storage and the sequencing
//! progress of one activated plan.
//!
//! Navigate's `FlightPlan` and `PlanExecution` borrow a caller-owned
//! waypoint slice (Navigate ADR-0015). The mission engine lives for a
//! whole flight and is moved between owners, so it cannot also hold a
//! borrow of its own storage. This type owns the waypoints and keeps the
//! two values that a `PlanExecution` changes: the active leg index and
//! the completion flag. Each operation builds the borrowed views over the
//! owned slice, runs, and keeps the new progress.

use navigate_contract::{FlightPlan, GeodeticPosition, Ident, PlanRole, TurnType, Waypoint};
use navigate_route::{ExecutionConfig, Leg, PlanExecution, SequenceEvent};

use crate::body;
use crate::error::MissionBuildError;

#[cfg(test)]
mod tests;

/// How a plan ends after its last waypoint sequences.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlanEnd {
    /// The plan completes when the last waypoint sequences.
    Sequenced,
    /// The vehicle flies on to the last waypoint and the plan completes
    /// within `radius_m` of it: horizontally, and also vertically when
    /// `vertical` is set.
    Arrive { radius_m: f64, vertical: bool },
}

/// One activated plan with its own waypoint storage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MissionRoute {
    id: Ident,
    role: PlanRole,
    waypoints: Vec<Waypoint>,
    config: ExecutionConfig,
    active_index: usize,
    complete: bool,
    end: PlanEnd,
    arrived: bool,
    /// Whether a first waypoint the vehicle starts past is passed rather
    /// than turned back for; see [`Self::pass_start`].
    pass_first_behind: bool,
    /// Whether [`Self::pass_start`] has judged the start.
    start_judged: bool,
}

impl MissionRoute {
    /// Validates and activates the plan. `name` is the Navigate plan
    /// identifier; the Pilotage plan identifier in the mission document
    /// is separate. The
    /// plan is on [`body::ELLIPSOID`], and fly-by turns use
    /// [`body::SURFACE_GRAVITY_MPS2`].
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::NavigationIdent`] for a `name` that is not an
    /// identifier, and every refusal of `PlanExecution::new`.
    pub(crate) fn activate(
        name: &str,
        role: PlanRole,
        waypoints: Vec<Waypoint>,
        config: ExecutionConfig,
    ) -> Result<Self, MissionBuildError> {
        let id = Ident::try_new(name).map_err(|source| MissionBuildError::NavigationIdent {
            ident: name.to_owned(),
            source,
        })?;
        let route = Self {
            id,
            role,
            waypoints,
            config: config.with_surface_gravity_mps2(body::SURFACE_GRAVITY_MPS2),
            active_index: 0,
            complete: false,
            end: PlanEnd::Sequenced,
            arrived: false,
            pass_first_behind: false,
            start_judged: false,
        };
        PlanExecution::new(route.plan(), route.config)?;
        Ok(route)
    }

    /// Sets how the plan ends after its last waypoint sequences.
    pub(crate) const fn with_end(mut self, end: PlanEnd) -> Self {
        self.end = end;
        self
    }

    /// Passes a first waypoint the vehicle starts past: see
    /// [`Self::pass_start`].
    pub(crate) const fn passing_a_first_waypoint_behind(mut self) -> Self {
        self.pass_first_behind = true;
        self
    }

    /// Judges the start of the plan, once, at the first position it flies
    /// from. A vehicle already on the first leg, past the perpendicular
    /// through the first waypoint and short of the second's, has passed
    /// the first waypoint: the first leg becomes the active one, and the
    /// vehicle joins it ahead instead of turning back. An airborne fixed
    /// wing turning back would fly a reversal at its route bank for a
    /// point it is already past. A first waypoint marked fly-over is still
    /// turned back for: the plan asks for the vehicle over it.
    ///
    /// Returns the index of the leg made active and the cross-track
    /// distance from the first leg, metres, positive right of it.
    pub(crate) fn pass_start(&mut self, position: &GeodeticPosition) -> Option<(usize, f64)> {
        if !self.pass_first_behind || self.start_judged {
            return None;
        }
        self.start_judged = true;
        let [first, second, ..] = self.waypoints.as_slice() else {
            return None;
        };
        if self.active_index != 0 || self.complete || first.turn != TurnType::FlyBy {
            return None;
        }
        let along_m = navigate_geodesy::along_track_m(
            position,
            &first.position,
            &second.position,
            body::GEODESY,
        )
        .ok()?;
        let length_m =
            navigate_geodesy::distance_m(&first.position, &second.position, body::GEODESY);
        if !(along_m > 0.0 && along_m < length_m) {
            return None;
        }
        let cross_track_m = navigate_geodesy::cross_track_m(
            position,
            &first.position,
            &second.position,
            body::GEODESY,
        )
        .ok()?;
        self.active_index = 1;
        Some((1, cross_track_m))
    }

    /// Whether the last waypoint sequenced and the vehicle still flies on
    /// to arrive at it.
    pub(crate) fn approaching(&self) -> bool {
        self.complete && matches!(self.end, PlanEnd::Arrive { .. }) && !self.arrived
    }

    /// The last waypoint.
    pub(crate) fn last_waypoint(&self) -> Option<&Waypoint> {
        self.waypoints.last()
    }

    /// The horizontal distance from `position` to the last waypoint.
    pub(crate) fn distance_to_last_m(&self, position: &GeodeticPosition) -> Option<f64> {
        let last = self.waypoints.last()?;
        Some(navigate_geodesy::distance_m(
            position,
            &last.position,
            body::GEODESY,
        ))
    }

    /// Records the arrival at the last waypoint when `position` is within
    /// the arrival radius of it; true at the arrival.
    pub(crate) fn arrive(&mut self, position: &GeodeticPosition) -> bool {
        let PlanEnd::Arrive { radius_m, vertical } = self.end else {
            return false;
        };
        let (Some(horizontal_m), Some(last)) =
            (self.distance_to_last_m(position), self.waypoints.last())
        else {
            return false;
        };
        let vertical_m = (position.altitude_m - last.position.altitude_m).abs();
        self.arrived =
            self.complete && horizontal_m <= radius_m && (!vertical || vertical_m <= radius_m);
        self.arrived
    }

    /// The bank the turns ahead of fly-by waypoints are sized at, radians.
    pub(crate) const fn bank_limit_rad(&self) -> f64 {
        self.config.bank_limit_rad
    }

    /// The courses into and out of the waypoint the active leg flies to,
    /// radians clockwise from true north: the turn ahead. `None` with no
    /// leg before it or after it.
    pub(crate) fn turn_ahead_courses(&self) -> Option<(f64, f64)> {
        if self.complete {
            return None;
        }
        let index = self.active_index;
        let previous = self.waypoints.get(index.checked_sub(1)?)?;
        let turn = self.waypoints.get(index)?;
        let next = self.waypoints.get(index.checked_add(1)?)?;
        let back = navigate_geodesy::initial_bearing_rad(&turn.position, &previous.position);
        let out = navigate_geodesy::initial_bearing_rad(&turn.position, &next.position);
        Some((back + core::f64::consts::PI, out))
    }

    /// The course of the last leg where it reaches the last waypoint,
    /// radians clockwise from true north; `None` for a plan of one
    /// waypoint, or a last leg too short to have a course.
    pub(crate) fn final_course_rad(&self) -> Option<f64> {
        let [.., previous, last] = self.waypoints.as_slice() else {
            return None;
        };
        let length_m =
            navigate_geodesy::distance_m(&previous.position, &last.position, body::GEODESY);
        if length_m.is_nan() || length_m <= 1.0 {
            return None;
        }
        let back = navigate_geodesy::initial_bearing_rad(&last.position, &previous.position);
        Some((back + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU))
    }

    /// The plan as a Navigate view over the owned waypoints.
    pub(crate) fn plan(&self) -> FlightPlan<'_> {
        FlightPlan::new(self.id, self.role, body::ELLIPSOID, &self.waypoints)
    }

    /// The waypoints in fly order.
    pub(crate) fn waypoints(&self) -> &[Waypoint] {
        &self.waypoints
    }

    /// Index of the waypoint that the active leg flies to.
    pub(crate) fn active_index(&self) -> usize {
        self.active_index
    }

    /// The waypoints from the active one to the end; empty when complete.
    pub(crate) fn remaining_waypoints(&self) -> &[Waypoint] {
        if self.complete {
            return &[];
        }
        self.waypoints.get(self.active_index..).unwrap_or_default()
    }

    /// The leg being flown, or `None` when the plan is complete.
    pub(crate) fn active_leg(&self) -> Option<Leg<'_>> {
        self.execution()?.active_leg()
    }

    /// Feeds one position sample to the sequencer and keeps the progress.
    pub(crate) fn advance(
        &mut self,
        position: &GeodeticPosition,
        groundspeed_mps: f64,
    ) -> SequenceEvent {
        let Some(mut execution) = self.execution() else {
            return SequenceEvent::None;
        };
        let event = execution.advance(position, groundspeed_mps);
        let (active_index, complete) = (execution.active_index(), execution.is_complete());
        self.active_index = active_index;
        self.complete = complete;
        event
    }

    /// A `PlanExecution` at the kept progress. `None` only if Navigate
    /// refuses to resume, which cannot occur: the plan and the config do
    /// not change after `activate`, and the kept progress is always a
    /// state that a `PlanExecution` reported, never one this type makes.
    fn execution(&self) -> Option<PlanExecution<'_>> {
        PlanExecution::resume(self.plan(), self.config, self.active_index, self.complete).ok()
    }
}
