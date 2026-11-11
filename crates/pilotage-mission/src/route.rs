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
}

impl MissionRoute {
    /// Validates and activates the plan. `kind` is the Navigate plan
    /// identifier, for example `mission`, `survey`, or `home`; the
    /// Pilotage plan identifier in the mission document is separate. The
    /// plan is on [`body::ELLIPSOID`], and fly-by turns use
    /// [`body::SURFACE_GRAVITY_MPS2`].
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::NavigationIdent`] for a `kind` that is not an
    /// identifier, and every refusal of `PlanExecution::new`.
    pub(crate) fn activate(
        kind: &str,
        role: PlanRole,
        waypoints: Vec<Waypoint>,
        config: ExecutionConfig,
    ) -> Result<Self, MissionBuildError> {
        let id = Ident::try_new(kind).map_err(|source| MissionBuildError::NavigationIdent {
            ident: kind.to_owned(),
            source,
        })?;
        let route = Self {
            id,
            role,
            waypoints,
            config: config.with_surface_gravity_mps2(body::SURFACE_GRAVITY_MPS2),
            active_index: 0,
            complete: false,
        };
        PlanExecution::new(route.plan(), route.config)?;
        Ok(route)
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

    /// A `PlanExecution` at the kept progress. `None` only if the owned
    /// plan stops activating, which cannot occur: the plan and the
    /// config do not change after `activate`.
    ///
    /// `PlanExecution` has no constructor that starts at a leg, so this
    /// replays the kept progress: one sample at each passed waypoint.
    /// A sample at a fix is at distance zero, which is less than the
    /// positive capture radius that activation requires, so each sample
    /// sequences exactly one leg.
    fn execution(&self) -> Option<PlanExecution<'_>> {
        let mut execution = PlanExecution::new(self.plan(), self.config).ok()?;
        let passed = if self.complete {
            self.waypoints.len()
        } else {
            self.active_index
        };
        for waypoint in self.waypoints.iter().take(passed) {
            if matches!(
                execution.advance(&waypoint.position, 0.0),
                SequenceEvent::None
            ) {
                return None;
            }
        }
        Some(execution)
    }
}
