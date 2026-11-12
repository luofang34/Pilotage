//! The owned mission route: the waypoint storage and the sequencing
//! progress of one activated plan.
//!
//! Navigate's `FlightPlan` and `PlanExecution` borrow a caller-owned
//! waypoint slice (Navigate ADR-0015). The mission engine lives for a
//! whole flight and is moved between owners, so it cannot also hold a
//! borrow of its own storage. This type owns the waypoints and keeps the
//! portable [`ExecutionState`] that `PlanExecution` updates, including
//! active hold circuit progress. Each operation builds the borrowed views
//! over the owned slice, runs, and keeps the new progress.

use navigate_contract::{FlightPlan, GeodeticPosition, Ident, LegPath, PlanRole, Waypoint};
use navigate_route::{
    ExecutionConfig, ExecutionState, HoldStatus, Leg, PlanExecution, SequenceEvent,
};

use crate::body;
use crate::engine::{FlightPhase, LegRole, PhaseMachine};
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
    roles: Vec<Option<LegRole>>,
    rnp_nm: Vec<Option<f64>>,
    config: ExecutionConfig,
    state: ExecutionState,
    end: PlanEnd,
    arrived: bool,
    /// Whether a first waypoint the vehicle is past is passed rather
    /// than turned back for; see [`Self::pass_start`].
    pass_first_behind: bool,
}

impl MissionRoute {
    /// Validates and activates the plan. `name` is the Navigate plan
    /// identifier; the Pilotage plan identifier in the mission document
    /// is separate. The plan is on [`body::ELLIPSOID`], and fly-by and
    /// hold turns use [`body::SURFACE_GRAVITY_MPS2`].
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
        let config = config.with_surface_gravity_mps2(body::SURFACE_GRAVITY_MPS2);
        let state = {
            let plan = FlightPlan::new(id, role, body::ELLIPSOID, &waypoints);
            PlanExecution::new(plan, config)?.state()
        };
        let count = waypoints.len();
        Ok(Self {
            id,
            role,
            waypoints,
            roles: vec![None; count],
            rnp_nm: vec![None; count],
            config,
            state,
            end: PlanEnd::Sequenced,
            arrived: false,
            pass_first_behind: false,
        })
    }

    /// Attaches per-leg procedure roles and optional RNP overrides.
    pub(crate) fn with_leg_metadata(
        mut self,
        roles: Vec<Option<LegRole>>,
        rnp_nm: Vec<Option<f64>>,
    ) -> Self {
        self.roles = roles;
        self.rnp_nm = rnp_nm;
        self
    }

    /// Evaluates the operational [`FlightPhase`] at the active leg.
    pub(crate) fn flight_phase(&self) -> FlightPhase {
        let index = if self.state.is_complete() {
            self.waypoints.len().saturating_sub(1)
        } else {
            self.state.active_index()
        };
        PhaseMachine::evaluate(&self.roles, index)
    }

    /// Procedure-published RNP override on the active leg, in nautical
    /// miles, when one was declared.
    pub(crate) fn active_rnp_nm(&self) -> Option<f64> {
        let index = if self.state.is_complete() {
            self.waypoints.len().saturating_sub(1)
        } else {
            self.state.active_index()
        };
        self.rnp_nm.get(index).copied().flatten()
    }

    /// Observable progress when the active leg is an in-route hold.
    pub(crate) const fn hold_status(&self) -> Option<HoldStatus> {
        self.state.hold_status()
    }

    /// Arms manual exit on an active `HM` (`HoldManual`) leg; returns
    /// `true` when the active leg is `HoldManual`.
    pub(crate) fn exit_hold(&mut self) -> bool {
        let Some(mut execution) = self.execution() else {
            return false;
        };
        let armed = execution.exit_hold();
        self.state = execution.state();
        armed
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

    /// Passes the first waypoint once the vehicle is at or beyond the
    /// perpendicular through it on the first leg, as an FMS sequences the
    /// initial fix of a plan it starts on or past. Initial hold legs are
    /// never skipped.
    ///
    /// Returns the index of the leg made active and the cross-track
    /// distance from the first leg, metres, positive right of it.
    pub(crate) fn pass_start(&mut self, position: &GeodeticPosition) -> Option<(usize, f64)> {
        if !self.pass_first_behind || self.state.active_index() != 0 || self.state.is_complete() {
            return None;
        }
        let [first, second, ..] = self.waypoints.as_slice() else {
            return None;
        };
        if !matches!(
            first.path,
            None | Some(LegPath::InitialFix | LegPath::DirectToFix)
        ) {
            return None;
        }
        // At or beyond the perpendicular through the first waypoint.
        navigate_geodesy::along_track_m(position, &first.position, &second.position, body::GEODESY)
            .ok()
            .filter(|along_m| *along_m >= 0.0)?;
        let cross_track_m = navigate_geodesy::cross_track_m(
            position,
            &first.position,
            &second.position,
            body::GEODESY,
        )
        .ok()?;
        self.state = PlanExecution::resume(self.plan(), self.config, 1, false)
            .ok()?
            .state();
        Some((1, cross_track_m))
    }

    /// Whether the last waypoint sequenced and the vehicle still flies on
    /// to arrive at it.
    pub(crate) fn approaching(&self) -> bool {
        self.state.is_complete() && matches!(self.end, PlanEnd::Arrive { .. }) && !self.arrived
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
        self.arrived = self.state.is_complete()
            && horizontal_m <= radius_m
            && (!vertical || vertical_m <= radius_m);
        self.arrived
    }

    /// The bank the turns ahead of fly-by waypoints are sized at, radians.
    pub(crate) const fn bank_limit_rad(&self) -> f64 {
        self.config.bank_limit_rad
    }

    /// The courses into and out of the waypoint the active leg flies to,
    /// radians clockwise from true north: the turn ahead. `None` with no
    /// leg before or after it, or when either leg is a hold.
    pub(crate) fn turn_ahead_courses(&self) -> Option<(f64, f64)> {
        if self.state.is_complete() {
            return None;
        }
        let index = self.state.active_index();
        let turn = self.waypoints.get(index)?;
        if turn.path.and_then(|p| p.hold()).is_some() {
            return None;
        }
        let next = self.waypoints.get(index.checked_add(1)?)?;
        if next.path.and_then(|p| p.hold()).is_some() {
            return None;
        }
        let into = match turn.path {
            Some(
                LegPath::CourseToFix { course_rad, .. }
                | LegPath::CourseToAltitude { course_rad, .. },
            ) => course_rad,
            _ => {
                let previous = self.waypoints.get(index.checked_sub(1)?)?;
                navigate_geodesy::initial_bearing_rad(&turn.position, &previous.position)
                    + core::f64::consts::PI
            }
        };
        let out = match next.path {
            Some(
                LegPath::CourseToFix { course_rad, .. }
                | LegPath::CourseToAltitude { course_rad, .. },
            ) => course_rad,
            _ => navigate_geodesy::initial_bearing_rad(&turn.position, &next.position),
        };
        Some((into, out))
    }

    /// The course of the last leg where it reaches the last waypoint,
    /// radians clockwise from true north; `None` for a plan of one
    /// waypoint with no published course, or a last leg too short to have
    /// a course.
    pub(crate) fn final_course_rad(&self) -> Option<f64> {
        let last = self.waypoints.last()?;
        if let Some(hold) = last.path.and_then(|p| p.hold()) {
            return Some(hold.inbound_course_rad.rem_euclid(core::f64::consts::TAU));
        }
        if let Some(
            LegPath::CourseToFix { course_rad, .. } | LegPath::CourseToAltitude { course_rad, .. },
        ) = last.path
        {
            return Some(course_rad.rem_euclid(core::f64::consts::TAU));
        }
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
    pub(crate) const fn active_index(&self) -> usize {
        self.state.active_index()
    }

    /// The waypoints from the active one to the end; empty when complete.
    pub(crate) fn remaining_waypoints(&self) -> &[Waypoint] {
        if self.state.is_complete() {
            return &[];
        }
        self.waypoints
            .get(self.state.active_index()..)
            .unwrap_or_default()
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
        self.state = execution.state();
        event
    }

    /// A `PlanExecution` at the kept progress. `None` only if Navigate
    /// refuses to resume, which cannot occur: the plan and the config do
    /// not change after `activate`, and the kept progress is always a
    /// state that a `PlanExecution` reported.
    fn execution(&self) -> Option<PlanExecution<'_>> {
        PlanExecution::resume_state(self.plan(), self.config, self.state).ok()
    }
}
