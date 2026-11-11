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

use navigate_contract::{
    AltitudeConstraint, FlightPlan, GeodeticPosition, Ident, PlanRole, Waypoint,
};
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
    /// The plan completes when the last waypoint sequences, and the
    /// vehicle then orbits it at `radius_m` and `altitude_m`.
    Orbit { radius_m: f64, altitude_m: f64 },
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
        };
        PlanExecution::new(route.plan(), route.config)?;
        Ok(route)
    }

    /// Sets how the plan ends after its last waypoint sequences.
    pub(crate) const fn with_end(mut self, end: PlanEnd) -> Self {
        self.end = end;
        self
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

    /// The orbit after the plan: its centre at the orbit altitude, and its
    /// radius. A plan without an orbit end orbits its last waypoint at the
    /// capture radius.
    pub(crate) fn terminal_orbit(&self) -> Option<(Waypoint, f64)> {
        let mut centre = *self.waypoints.last()?;
        match self.end {
            PlanEnd::Orbit {
                radius_m,
                altitude_m,
            } => {
                centre.position.altitude_m = altitude_m;
                centre.altitude = Some(AltitudeConstraint::At(altitude_m));
                Some((centre, radius_m))
            }
            PlanEnd::Sequenced | PlanEnd::Arrive { .. } => {
                Some((centre, self.config.capture_radius_m))
            }
        }
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
