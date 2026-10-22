//! Operational mission-document generation and flight-plan identity.

use navigate_contract::{AltitudeConstraint, FlightPlan, PlanRole, TurnType, Waypoint};
use pilotage_mission_core::{
    Comparison, Digest, ExecutionPolicy, ExecutionTarget, FlightAction, FlightPlanReference,
    MissionAction, MissionCapability, MissionCondition, MissionDocument, MissionPhase,
    NavigationCondition, NavigationDataIdentity,
};
use sha2::{Digest as _, Sha256};

use crate::MissionConfig;
use crate::error::MissionBuildError;
use crate::policy::{
    ARM_PHASE_DEADLINE_NS, CLIMB_PHASE_DEADLINE_NS, FOLLOW_PLAN_PHASE_DEADLINE_NS,
    OPERATIONAL_RECEIPT_TIMEOUT_NS, OPERATIONAL_RETRY_LIMIT,
};

pub(super) const ARM_PHASE_ID: &str = "arm";
pub(super) const CLIMB_PHASE_ID: &str = "climb";
pub(super) const FOLLOW_PLAN_PHASE_ID: &str = "follow-plan";

/// Altitude margin below the target that completes the climb, in meters.
pub(super) const CLIMB_CAPTURE_MARGIN_M: f64 = 1.0;

const PLAN_DIGEST_DOMAIN: &[u8] = b"pilotage.flight-plan.v1\0";
/// Revision prefix of a mission built from a navdata route.
pub(super) const ROUTE_REVISION_PREFIX: &str = "operational-route-v1";
/// Revision prefix of a mission built from an area survey request.
pub(super) const SURVEY_REVISION_PREFIX: &str = "operational-survey-v1";

pub(super) fn build_document(
    plan: &FlightPlan,
    config: &MissionConfig,
    navigation_data_identity: NavigationDataIdentity,
    revision_prefix: &str,
) -> Result<(MissionDocument, FlightPlanReference), MissionBuildError> {
    let plan_content_digest = plan_digest(plan)?;
    let plan_reference = FlightPlanReference {
        plan_id: plan.id.clone(),
        plan_content_digest,
        navigation_data_identity: navigation_data_identity.clone(),
    };
    let revision_id = format!("{revision_prefix}:{plan_content_digest}");
    let document = MissionDocument::new(
        revision_id,
        navigation_data_identity,
        ExecutionPolicy {
            target: config.execution_target,
            retry_limit: OPERATIONAL_RETRY_LIMIT,
            receipt_timeout_ns: OPERATIONAL_RECEIPT_TIMEOUT_NS,
        },
        phases(config, plan_reference.clone()),
    )?;
    Ok((document, plan_reference))
}

/// The phases the mission flies. A simulator's vehicle is armed by the
/// mission; a real vehicle is armed and launched by its pilot, so its
/// mission starts from the hover with the climb (ADR-0051).
fn phases(config: &MissionConfig, plan: FlightPlanReference) -> Vec<MissionPhase> {
    let mut phases = Vec::with_capacity(3);
    if config.execution_target == ExecutionTarget::Simulator {
        phases.push(arm_phase());
    }
    // The first phase waits for a valid solution; later phases follow on.
    let first = phases.is_empty();
    phases.push(climb_phase(config, first));
    phases.push(follow_plan_phase(plan));
    phases
}

fn arm_phase() -> MissionPhase {
    MissionPhase {
        id: ARM_PHASE_ID.to_owned(),
        required_capabilities: vec![
            MissionCapability::SimulatorTime,
            MissionCapability::ArmDisarm,
            MissionCapability::NavigationState,
        ],
        entry_conditions: vec![MissionCondition::Navigation(
            NavigationCondition::GuidanceValid { expected: true },
        )],
        action: MissionAction::Flight(FlightAction::Arm {}),
        cleanup_actions: Vec::new(),
        completion_conditions: vec![MissionCondition::Always {}],
        abort_conditions: Vec::new(),
        simulator_time_deadline_ns: ARM_PHASE_DEADLINE_NS,
    }
}

fn climb_phase(config: &MissionConfig, first: bool) -> MissionPhase {
    let target_altitude_m = config.anchor.altitude_m + config.cruise_height_m;
    let entry = if first {
        MissionCondition::Navigation(NavigationCondition::GuidanceValid { expected: true })
    } else {
        MissionCondition::Always {}
    };
    MissionPhase {
        id: CLIMB_PHASE_ID.to_owned(),
        required_capabilities: vec![
            MissionCapability::SimulatorTime,
            MissionCapability::FlightControl,
            MissionCapability::NavigationState,
        ],
        entry_conditions: vec![entry],
        action: MissionAction::Flight(FlightAction::Climb { target_altitude_m }),
        cleanup_actions: Vec::new(),
        completion_conditions: vec![MissionCondition::Navigation(
            NavigationCondition::Altitude {
                comparison: Comparison::GreaterOrEqual,
                value_m: target_altitude_m - CLIMB_CAPTURE_MARGIN_M,
            },
        )],
        abort_conditions: Vec::new(),
        simulator_time_deadline_ns: CLIMB_PHASE_DEADLINE_NS,
    }
}

fn follow_plan_phase(plan: FlightPlanReference) -> MissionPhase {
    MissionPhase {
        id: FOLLOW_PLAN_PHASE_ID.to_owned(),
        required_capabilities: vec![
            MissionCapability::SimulatorTime,
            MissionCapability::FlightPlan,
            MissionCapability::NavigationState,
        ],
        entry_conditions: vec![MissionCondition::Always {}],
        action: MissionAction::Flight(FlightAction::FollowPlan { plan }),
        cleanup_actions: Vec::new(),
        completion_conditions: vec![MissionCondition::Navigation(
            NavigationCondition::PlanComplete { expected: true },
        )],
        abort_conditions: Vec::new(),
        simulator_time_deadline_ns: FOLLOW_PLAN_PHASE_DEADLINE_NS,
    }
}

fn plan_digest(plan: &FlightPlan) -> Result<Digest, MissionBuildError> {
    let mut hasher = Sha256::new();
    hasher.update(PLAN_DIGEST_DOMAIN);
    hash_string(&mut hasher, &plan.id);
    let role = match plan.role {
        PlanRole::Mission => 1,
        PlanRole::LossOfComm => 2,
        PlanRole::Contingency => 3,
        other => return Err(unsupported("role", other)),
    };
    hasher.update([role]);
    hash_len(&mut hasher, plan.waypoints.len());
    for waypoint in &plan.waypoints {
        hash_waypoint(&mut hasher, waypoint)?;
    }
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

fn hash_waypoint(hasher: &mut Sha256, waypoint: &Waypoint) -> Result<(), MissionBuildError> {
    hash_string(hasher, &waypoint.ident);
    hash_f64(hasher, waypoint.position.latitude_rad);
    hash_f64(hasher, waypoint.position.longitude_rad);
    hash_f64(hasher, waypoint.position.altitude_m);
    hash_altitude(hasher, waypoint.altitude)?;
    let turn = match waypoint.turn {
        TurnType::FlyBy => 1,
        TurnType::FlyOver => 2,
        other => return Err(unsupported("waypoint.turn", other)),
    };
    hasher.update([turn]);
    hash_optional_f64(hasher, waypoint.max_speed_mps);
    hash_optional_f64(hasher, waypoint.gradient);
    Ok(())
}

fn hash_altitude(
    hasher: &mut Sha256,
    constraint: Option<AltitudeConstraint>,
) -> Result<(), MissionBuildError> {
    match constraint {
        None => hasher.update([0]),
        Some(AltitudeConstraint::At(value)) => hash_tagged_f64(hasher, 1, value),
        Some(AltitudeConstraint::AtOrAbove(value)) => hash_tagged_f64(hasher, 2, value),
        Some(AltitudeConstraint::AtOrBelow(value)) => hash_tagged_f64(hasher, 3, value),
        Some(AltitudeConstraint::Window { lower_m, upper_m }) => {
            hasher.update([4]);
            hash_f64(hasher, lower_m);
            hash_f64(hasher, upper_m);
        }
        Some(other) => return Err(unsupported("waypoint.altitude", other)),
    }
    Ok(())
}

fn hash_optional_f64(hasher: &mut Sha256, value: Option<f64>) {
    match value {
        Some(value) => hash_tagged_f64(hasher, 1, value),
        None => hasher.update([0]),
    }
}

fn hash_tagged_f64(hasher: &mut Sha256, tag: u8, value: f64) {
    hasher.update([tag]);
    hash_f64(hasher, value);
}

fn hash_f64(hasher: &mut Sha256, value: f64) {
    hasher.update(value.to_bits().to_be_bytes());
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hash_len(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_len(hasher: &mut Sha256, len: usize) {
    hasher.update(u64::try_from(len).unwrap_or(u64::MAX).to_be_bytes());
}

fn unsupported(field: &'static str, value: impl core::fmt::Debug) -> MissionBuildError {
    MissionBuildError::PlanIdentity {
        field,
        value: format!("{value:?}"),
    }
}
