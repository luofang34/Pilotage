//! A mission over a waypoint list: the plan is the list itself.

use navigate_contract::{
    AltitudeConstraint, GeodeticPosition, Ident, PlanRole, TurnType, Waypoint,
};
use navigate_route::ExecutionConfig;
use pilotage_mission_core::NavigationDataIdentity;
use pilotage_protocol::VehicleClass;

use super::{MissionEngine, document};
use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::route::{MissionRoute, PlanEnd};
use crate::waypoints::{
    PlannedWaypoint, WaypointPlan, WaypointPlanRecord, WaypointTurn, capture_radius_for,
    true_airspeed_for, turn_radius_for,
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
        let cruise_mps = config.cruise_mps.min(config.limits.max_horizontal_mps);
        let capture_radius_m = plan
            .capture_radius_m
            .unwrap_or_else(|| capture_radius_for(cruise_mps, config.capture_time_s));
        let end = plan_end(plan, &config, &waypoints, cruise_mps, capture_radius_m);
        let mut execution = ExecutionConfig::new(capture_radius_m);
        execution.bank_limit_rad = config.route_bank_deg.to_radians();
        let route = MissionRoute::activate(&name, PlanRole::Mission, waypoints, execution)?
            .with_end(end)
            .passing_a_first_waypoint_behind();
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

/// How the plan ends for the configured vehicle: a vehicle that can stop
/// arrives at the last waypoint, checked vertically too for an aircraft;
/// one that cannot stop orbits it.
fn plan_end(
    plan: &WaypointPlan,
    config: &MissionConfig,
    waypoints: &[Waypoint],
    cruise_mps: f64,
    capture_radius_m: f64,
) -> PlanEnd {
    if config.can_stop() {
        let vertical = matches!(
            config.vehicle_class,
            VehicleClass::Multicopter | VehicleClass::Vtol
        );
        return PlanEnd::Arrive {
            radius_m: plan.arrival_radius_m,
            vertical,
        };
    }
    let orbit = config.terminal_orbit;
    let last_altitude_m = waypoints
        .last()
        .map_or(config.anchor.altitude_m, |last| last.position.altitude_m);
    let altitude_m = orbit.altitude_m.unwrap_or(last_altitude_m);
    // A fixed wing holds its indicated airspeed, and the circle it can fly
    // is set by its true airspeed in the orbit's thinner air.
    let orbit_speed_mps = true_airspeed_for(cruise_mps, altitude_m);
    PlanEnd::Orbit {
        radius_m: orbit.radius_m.unwrap_or_else(|| {
            turn_radius_for(orbit_speed_mps, config.route_bank_deg, capture_radius_m)
        }),
        altitude_m,
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
