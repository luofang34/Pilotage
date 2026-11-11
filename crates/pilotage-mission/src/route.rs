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
    /// The radius of the orbit around the last waypoint after the plan
    /// completes, when the plan flies on to pass its last waypoint.
    terminal_orbit_m: Option<f64>,
    passed_last: bool,
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
            terminal_orbit_m: None,
            passed_last: false,
        };
        PlanExecution::new(route.plan(), route.config)?;
        Ok(route)
    }

    /// The route flies on along its last leg after the sequencer captures
    /// the last waypoint, until the vehicle passes it. A capture radius
    /// sized for turns would otherwise end the plan that far short of its
    /// last waypoint. After the pass, a vehicle that cannot hover orbits
    /// the last waypoint at `orbit_radius_m`.
    pub(crate) fn with_terminal_overfly(mut self, orbit_radius_m: f64) -> Self {
        self.terminal_orbit_m = Some(orbit_radius_m);
        self
    }

    /// Whether the sequencer completed the plan and the vehicle has not
    /// yet passed the last waypoint.
    pub(crate) fn approaching(&self) -> bool {
        self.complete && self.terminal_orbit_m.is_some() && !self.passed_last
    }

    /// The last leg: its start, if the plan has one, and the last waypoint.
    pub(crate) fn last_leg(&self) -> Option<(Option<GeodeticPosition>, Waypoint)> {
        let (last, earlier) = self.waypoints.split_last()?;
        Some((earlier.last().map(|from| from.position), *last))
    }

    /// The orbit around the last waypoint: its centre and radius. A plan
    /// without a terminal overfly orbits at its capture radius.
    pub(crate) fn terminal_orbit(&self) -> Option<(Waypoint, f64)> {
        let radius_m = self
            .terminal_orbit_m
            .unwrap_or(self.config.capture_radius_m);
        Some((*self.waypoints.last()?, radius_m))
    }

    /// Records the pass of the last waypoint when `position` lies beyond
    /// it along the last leg; true at the pass. A plan of one waypoint
    /// passes it at its capture.
    pub(crate) fn pass_last(&mut self, position: &GeodeticPosition) -> bool {
        if !self.approaching() {
            return false;
        }
        let Some((from, last)) = self.last_leg() else {
            return false;
        };
        self.passed_last = from.is_none_or(|from| {
            navigate_geodesy::along_track_m(position, &from, &last.position, body::GEODESY)
                .is_ok_and(|along| {
                    along >= navigate_geodesy::distance_m(&from, &last.position, body::GEODESY)
                })
        });
        self.passed_last
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
