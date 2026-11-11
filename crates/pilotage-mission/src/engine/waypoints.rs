//! A mission over a waypoint list: the plan is the list itself.

use navigate_contract::{
    AltitudeConstraint, GeodeticPosition, Ident, PlanRole, TurnType, Waypoint,
};
use navigate_route::ExecutionConfig;
use pilotage_mission_core::NavigationDataIdentity;

use super::{MissionEngine, document};
use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::route::MissionRoute;
use crate::waypoints::{
    DEFAULT_MAX_BANK_DEG, PlannedWaypoint, WaypointPlan, WaypointPlanRecord, WaypointTurn,
    capture_radius_for,
};

impl MissionEngine {
    /// Builds an engine that flies `plan`: each waypoint at its own
    /// position and altitude, passed as its turn says. The plan's
    /// identity is its content digest; no navigation data take part.
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::EmptyWaypointList`] for no waypoints,
    /// [`MissionBuildError::WaypointPosition`] for a position or altitude
    /// that is not finite or not on the Earth,
    /// [`MissionBuildError::NavigationIdent`] for a name that is empty or
    /// longer than 32 bytes, and the activation, geodesy and document
    /// errors of [`Self::new`].
    pub fn from_waypoints(
        plan: &WaypointPlan,
        config: MissionConfig,
    ) -> Result<(Self, WaypointPlanRecord), MissionBuildError> {
        if plan.waypoints.is_empty() {
            return Err(MissionBuildError::EmptyWaypointList);
        }
        let waypoints = plan
            .waypoints
            .iter()
            .enumerate()
            .map(|(index, planned)| waypoint(index, planned))
            .collect::<Result<Vec<_>, _>>()?;
        let digest = plan.digest();
        let idents: Vec<String> = waypoints
            .iter()
            .map(|waypoint| waypoint.ident.as_str().to_owned())
            .collect();
        let identity = NavigationDataIdentity {
            cycle: "waypoints".to_owned(),
            snapshot_id: format!("waypoints:{digest}"),
            snapshot_digest: digest,
        };
        let plan_id = format!("waypoints:{digest}");
        let name =
            document::navigate_plan_name("waypoints", &plan_id, PlanRole::Mission, &waypoints)?;
        let capture_radius_m = plan.capture_radius_m.unwrap_or_else(|| {
            capture_radius_for(
                config.cruise_mps.min(config.limits.max_horizontal_mps),
                DEFAULT_MAX_BANK_DEG,
            )
        });
        let mut execution = ExecutionConfig::new(capture_radius_m);
        execution.bank_limit_rad = DEFAULT_MAX_BANK_DEG.to_radians();
        let route = MissionRoute::activate(&name, PlanRole::Mission, waypoints, execution)?
            .with_terminal_overfly(capture_radius_m);
        let engine = Self::from_route(
            route,
            &plan_id,
            config,
            identity,
            document::WAYPOINTS_REVISION_PREFIX,
            &[],
        )?;
        let record = WaypointPlanRecord {
            digest,
            idents,
            capture_radius_m,
            mission_identity: engine.document.identity.clone(),
        };
        Ok((engine, record))
    }
}

/// One planned waypoint as a Navigate waypoint. Degrees convert to
/// radians here, once (ADR-0030).
fn waypoint(index: usize, planned: &PlannedWaypoint) -> Result<Waypoint, MissionBuildError> {
    let on_earth = planned.latitude_deg.is_finite()
        && planned.longitude_deg.is_finite()
        && planned.altitude_m.is_finite()
        && planned.latitude_deg.abs() <= 90.0
        && planned.longitude_deg.abs() <= 180.0;
    if !on_earth {
        return Err(MissionBuildError::WaypointPosition {
            index,
            latitude_deg: planned.latitude_deg,
            longitude_deg: planned.longitude_deg,
            altitude_m: planned.altitude_m,
        });
    }
    let name = planned
        .name
        .clone()
        .unwrap_or_else(|| format!("WP{}", index.wrapping_add(1)));
    let ident = Ident::try_new(&name).map_err(|source| MissionBuildError::NavigationIdent {
        ident: name,
        source,
    })?;
    let position = GeodeticPosition::new(
        planned.latitude_deg.to_radians(),
        planned.longitude_deg.to_radians(),
        planned.altitude_m,
    );
    let turn = match planned.turn {
        WaypointTurn::FlyBy => TurnType::FlyBy,
        WaypointTurn::FlyOver => TurnType::FlyOver,
    };
    Ok(Waypoint::new(ident, position)
        .with_altitude(AltitudeConstraint::At(planned.altitude_m))
        .with_turn(turn))
}
