//! A mission over a waypoint list: the plan is the list itself.

use core::f64::consts::TAU;

use navigate_contract::{
    CourseReference, GeodeticPosition, HoldGeometry, HoldLegLength, Ident, LegPath, PlanRole,
    TurnDirection, TurnType, Waypoint,
};
use navigate_route::ExecutionConfig;
use pilotage_mission_core::NavigationDataIdentity;
use pilotage_protocol::VehicleClass;
use pilotage_world::MagneticEpoch;

use super::{MissionEngine, document};
use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::route::{MissionRoute, PlanEnd};
use crate::waypoints::{
    PlannedCourseReference, PlannedHold, PlannedHoldLength, PlannedPath, PlannedWaypoint,
    WaypointPlan, WaypointPlanRecord, WaypointTurn, WaypointTurnDirection, capture_radius_for,
};

impl MissionEngine {
    /// Builds an engine that flies `plan`: each waypoint at its own
    /// position under its altitude constraint, passed as its turn says,
    /// on the leg path, mandatory turn direction, speed limit, gradient,
    /// procedure role, and RNP it declares.
    ///
    /// # Errors
    ///
    /// Returns [`MissionBuildError`] for an empty list, an invalid
    /// position, RNP, or identifier, a magnetic variation error, or any
    /// plan validation or activation refusal from Navigate.
    pub fn from_waypoints(
        plan: &WaypointPlan,
        config: MissionConfig,
    ) -> Result<(Self, WaypointPlanRecord), MissionBuildError> {
        if plan.waypoints.is_empty() {
            return Err(MissionBuildError::EmptyWaypointList);
        }
        let mut waypoints = Vec::with_capacity(plan.waypoints.len());
        let mut roles = Vec::with_capacity(plan.waypoints.len());
        let mut rnp_nm = Vec::with_capacity(plan.waypoints.len());
        for (index, planned) in plan.waypoints.iter().enumerate() {
            if let Some(rnp) = planned.rnp_nm
                && (!rnp.is_finite() || rnp <= 0.0)
            {
                return Err(MissionBuildError::WaypointRnp { index, rnp_nm: rnp });
            }
            waypoints.push(waypoint(index, planned, config.magnetic_epoch)?);
            roles.push(planned.role);
            rnp_nm.push(planned.rnp_nm);
        }
        let digest = plan.digest();
        let idents: Vec<String> = waypoints
            .iter()
            .map(|wp| wp.ident.as_str().to_owned())
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
        let end = plan_end(plan, &config);
        let mut execution = ExecutionConfig::new(capture_radius_m);
        execution.bank_limit_rad = config.route_bank_deg.to_radians();
        let route = MissionRoute::activate(&name, PlanRole::Mission, waypoints, execution)?
            .with_leg_metadata(roles, rnp_nm)
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

fn plan_end(plan: &WaypointPlan, config: &MissionConfig) -> PlanEnd {
    if !config.can_stop() {
        return PlanEnd::Sequenced;
    }
    let vertical = matches!(
        config.vehicle_class,
        VehicleClass::Multicopter | VehicleClass::Vtol
    );
    PlanEnd::Arrive {
        radius_m: plan.arrival_radius_m,
        vertical,
    }
}

/// Converts one planned waypoint into a Navigate [`Waypoint`], converting
/// degrees to radians once (ADR-0030) and resolving magnetic (`P4`) courses
/// against WMM2025 when `epoch` is configured.
fn waypoint(
    index: usize,
    planned: &PlannedWaypoint,
    epoch: Option<MagneticEpoch>,
) -> Result<Waypoint, MissionBuildError> {
    let altitude_m = planned.altitude.reference_m();
    let on_earth = planned.latitude_deg.is_finite()
        && planned.longitude_deg.is_finite()
        && altitude_m.is_finite()
        && planned.latitude_deg.abs() <= 90.0
        && planned.longitude_deg.abs() <= 180.0;
    if !on_earth {
        return Err(MissionBuildError::WaypointPosition {
            index,
            latitude_deg: planned.latitude_deg,
            longitude_deg: planned.longitude_deg,
            altitude_m,
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
        altitude_m,
    );
    let turn = match planned.turn {
        WaypointTurn::FlyBy => TurnType::FlyBy,
        WaypointTurn::FlyOver => TurnType::FlyOver,
    };
    let mut built = Waypoint::new(ident, position).with_turn(turn);
    built.altitude = planned.altitude.constraint();
    built.turn_direction = planned.turn_direction.map(turn_direction);
    built.path = planned
        .path
        .map(|path| leg_path(index, planned, altitude_m, path, epoch))
        .transpose()?;
    built.max_speed_mps = planned.max_speed_mps;
    built.gradient = planned.gradient;
    Ok(built)
}

const fn turn_direction(direction: WaypointTurnDirection) -> TurnDirection {
    match direction {
        WaypointTurnDirection::Left => TurnDirection::Left,
        WaypointTurnDirection::Right => TurnDirection::Right,
    }
}

fn leg_path(
    index: usize,
    planned: &PlannedWaypoint,
    altitude_m: f64,
    path: PlannedPath,
    epoch: Option<MagneticEpoch>,
) -> Result<LegPath, MissionBuildError> {
    match path {
        PlannedPath::InitialFix => Ok(LegPath::InitialFix),
        PlannedPath::TrackToFix => Ok(LegPath::TrackToFix),
        PlannedPath::DirectToFix => Ok(LegPath::DirectToFix),
        PlannedPath::CourseToFix {
            course_rad,
            reference,
        } => {
            let (course_rad, reference) =
                resolve_course(index, planned, altitude_m, course_rad, reference, epoch)?;
            Ok(LegPath::CourseToFix {
                course_rad,
                reference,
            })
        }
        PlannedPath::CourseToAltitude {
            course_rad,
            reference,
        } => {
            let (course_rad, reference) =
                resolve_course(index, planned, altitude_m, course_rad, reference, epoch)?;
            Ok(LegPath::CourseToAltitude {
                course_rad,
                reference,
            })
        }
        PlannedPath::HoldToAltitude(hold) => Ok(LegPath::HoldToAltitude {
            hold: build_hold(index, planned, altitude_m, hold, epoch)?,
        }),
        PlannedPath::HoldToFix(hold) => Ok(LegPath::HoldToFix {
            hold: build_hold(index, planned, altitude_m, hold, epoch)?,
        }),
        PlannedPath::HoldManual(hold) => Ok(LegPath::HoldManual {
            hold: build_hold(index, planned, altitude_m, hold, epoch)?,
        }),
    }
}

fn build_hold(
    index: usize,
    planned: &PlannedWaypoint,
    altitude_m: f64,
    hold: PlannedHold,
    epoch: Option<MagneticEpoch>,
) -> Result<HoldGeometry, MissionBuildError> {
    let (inbound_course_rad, reference) = resolve_course(
        index,
        planned,
        altitude_m,
        hold.inbound_course_rad,
        hold.reference,
        epoch,
    )?;
    let leg = match hold.length {
        PlannedHoldLength::Minutes(minutes) => HoldLegLength::Time {
            seconds: minutes * 60.0,
        },
        PlannedHoldLength::DistanceM(meters) => HoldLegLength::Distance { meters },
    };
    Ok(HoldGeometry::new(
        inbound_course_rad,
        reference,
        turn_direction(hold.turn),
        leg,
    ))
}

fn resolve_course(
    index: usize,
    planned: &PlannedWaypoint,
    altitude_m: f64,
    course_rad: f64,
    reference: PlannedCourseReference,
    epoch: Option<MagneticEpoch>,
) -> Result<(f64, CourseReference), MissionBuildError> {
    match reference {
        PlannedCourseReference::True => Ok((course_rad, CourseReference::True)),
        PlannedCourseReference::Magnetic => {
            if !course_rad.is_finite() || !(0.0..TAU).contains(&course_rad) {
                return Ok((course_rad, CourseReference::True));
            }
            let Some(epoch) = epoch else {
                return Ok((course_rad, CourseReference::Magnetic));
            };
            let field = pilotage_world::magnetic_field_at(
                planned.latitude_deg,
                planned.longitude_deg,
                altitude_m,
                epoch,
            )
            .map_err(|source| MissionBuildError::MagneticField {
                index,
                source: Box::new(source),
            })?;
            let mut true_rad = (course_rad + field.declination_rad()).rem_euclid(TAU);
            if true_rad >= TAU {
                true_rad = 0.0;
            }
            Ok((true_rad, CourseReference::True))
        }
    }
}
