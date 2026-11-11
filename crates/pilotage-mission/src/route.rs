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

use navigate_contract::{FlightPlan, GeodeticPosition, Ident, PlanRole, Waypoint};
use navigate_route::{ExecutionConfig, Leg, PlanExecution, SequenceEvent};

use crate::body;
use crate::error::MissionBuildError;

#[cfg(test)]
mod tests;

/// One activated plan with its own waypoint storage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MissionRoute {
    id: Ident,
    role: PlanRole,
    waypoints: Vec<Waypoint>,
    config: ExecutionConfig,
    active_index: usize,
    complete: bool,
    /// The distance to the last waypoint at which the plan completes,
    /// when the plan flies on to it after its capture.
    arrival_radius_m: Option<f64>,
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
            arrival_radius_m: None,
            arrived: false,
        };
        PlanExecution::new(route.plan(), route.config)?;
        Ok(route)
    }

    /// The route flies on to its last waypoint after the sequencer
    /// captures it, until the vehicle is within `radius_m` of it
    /// horizontally and vertically. A capture radius sized for turns
    /// would otherwise end the plan that far short of its last waypoint.
    pub(crate) fn with_arrival_radius(mut self, radius_m: f64) -> Self {
        self.arrival_radius_m = Some(radius_m);
        self
    }

    /// Whether the sequencer completed the plan and the vehicle still
    /// flies on to the last waypoint.
    pub(crate) fn approaching(&self) -> bool {
        self.complete && self.arrival_radius_m.is_some() && !self.arrived
    }

    /// The last waypoint, which an approaching route flies to.
    pub(crate) fn last_waypoint(&self) -> Option<&Waypoint> {
        self.waypoints.last()
    }

    /// Records the arrival at the last waypoint when `position` is within
    /// the arrival radius of it; true at the arrival.
    pub(crate) fn arrive(&mut self, position: &GeodeticPosition) -> bool {
        let (Some(radius_m), Some(last)) = (self.arrival_radius_m, self.waypoints.last()) else {
            return false;
        };
        if !self.approaching() {
            return false;
        }
        let horizontal_m = navigate_geodesy::distance_m(position, &last.position, body::GEODESY);
        let vertical_m = (position.altitude_m - last.position.altitude_m).abs();
        self.arrived = horizontal_m <= radius_m && vertical_m <= radius_m;
        self.arrived
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
