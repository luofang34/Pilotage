//! Operational mission-document generation and identity tests.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::policy::{
    ARM_PHASE_DEADLINE_NS, CLIMB_PHASE_DEADLINE_NS, FOLLOW_PLAN_PHASE_DEADLINE_NS,
    OPERATIONAL_RECEIPT_TIMEOUT_NS, OPERATIONAL_RETRY_LIMIT, OPERATIONAL_WALL_DEADLINE_NS,
};
use pilotage_mission::{MissionConfig, MissionEngine, decode_snapshot};
use pilotage_mission_core::{
    DirectiveReceipt, EngineStart, EngineState, ExecutionTarget, FlightAction, MissionAction,
    MissionCondition, MissionEngine as CoreMissionEngine, MissionObservation, MissionTerminal,
    NavigationCondition, NavigationObservation, ReceiptResult, TickInput, VehicleObservation,
    WallDeadline,
};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};

fn build(cruise_height_m: f64) -> (MissionEngine, pilotage_mission::MissionPlanRecord) {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let anchor = GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    );
    let mut config = MissionConfig::new(
        fixture::DEMO_ROUTE.to_owned(),
        anchor,
        ClockDomainId::new(7),
    );
    config.cruise_height_m = cruise_height_m;
    MissionEngine::new(&snapshot, provenance, config).expect("mission builds")
}

#[test]
fn route_config_produces_the_three_bounded_action_phases() {
    let (engine, record) = build(15.0);
    let document = engine.mission_document();
    document.validate().expect("document validates");
    document
        .to_canonical_json()
        .expect("document has a verified digest");
    assert_eq!(record.mission_identity, document.identity);
    assert_eq!(
        document.execution_policy.retry_limit,
        OPERATIONAL_RETRY_LIMIT
    );
    assert_eq!(
        document.execution_policy.receipt_timeout_ns,
        OPERATIONAL_RECEIPT_TIMEOUT_NS
    );
    assert_eq!(OPERATIONAL_WALL_DEADLINE_NS, 24 * 60 * 60 * 1_000_000_000);
    assert_eq!(document.phases.len(), 3);
    assert_eq!(document.phases[0].id, "arm");
    assert_eq!(document.phases[1].id, "climb");
    assert_eq!(document.phases[2].id, "follow-plan");
    assert_eq!(
        document.phases[0].simulator_time_deadline_ns,
        ARM_PHASE_DEADLINE_NS
    );
    assert_eq!(
        document.phases[1].simulator_time_deadline_ns,
        CLIMB_PHASE_DEADLINE_NS
    );
    assert_eq!(
        document.phases[2].simulator_time_deadline_ns,
        FOLLOW_PLAN_PHASE_DEADLINE_NS
    );
    assert!(document.phases.iter().all(|phase| {
        !matches!(
            phase.action,
            MissionAction::Flight(FlightAction::MaintainTarget {})
        )
    }));
}

#[test]
fn navigation_solution_altitude_and_plan_completion_are_document_conditions() {
    let (engine, _) = build(15.0);
    let phases = &engine.mission_document().phases;
    assert!(matches!(
        phases[0].entry_conditions.as_slice(),
        [MissionCondition::Navigation(
            NavigationCondition::GuidanceValid { expected: true }
        )]
    ));
    assert!(matches!(
        phases[1].action,
        MissionAction::Flight(FlightAction::Climb {
            target_altitude_m: 515.0
        })
    ));
    assert!(matches!(
        phases[1].completion_conditions.as_slice(),
        [MissionCondition::Navigation(
            NavigationCondition::Altitude { value_m: 514.0, .. }
        )]
    ));
    assert!(matches!(
        phases[2].completion_conditions.as_slice(),
        [MissionCondition::Navigation(
            NavigationCondition::PlanComplete { expected: true }
        )]
    ));
}

#[test]
fn built_plan_and_navdata_identities_bind_the_document() {
    let (first, first_record) = build(15.0);
    let (higher, _) = build(20.0);
    let first_document = first.mission_document();
    let higher_document = higher.mission_document();
    let MissionAction::Flight(FlightAction::FollowPlan { plan: first_plan }) =
        &first_document.phases[2].action
    else {
        panic!("the final action follows the plan");
    };
    let MissionAction::Flight(FlightAction::FollowPlan { plan: higher_plan }) =
        &higher_document.phases[2].action
    else {
        panic!("the final action follows the plan");
    };
    assert_eq!(
        first_plan.navigation_data_identity,
        first_record.provenance.navigation_data_identity
    );
    assert_eq!(
        first_document.identity.navigation_data_identity,
        first_plan.navigation_data_identity
    );
    assert!(!first_plan.plan_content_digest.is_zero());
    assert_ne!(
        first_plan.plan_content_digest, higher_plan.plan_content_digest,
        "a changed built altitude profile changes the plan digest"
    );
    assert_ne!(
        first_document.identity.content_digest,
        higher_document.identity.content_digest
    );
}

#[test]
fn plan_completion_before_the_follow_deadline_is_complete() {
    let (engine, _) = build(15.0);
    let document = engine.mission_document().clone();
    let start_ns = 1_000_000_000;
    let mut core = CoreMissionEngine::start(
        document.clone(),
        EngineStart {
            host_target: ExecutionTarget::Simulator,
            simulator_time_ns: start_ns,
            wall_time_ns: start_ns,
            wall_deadline: WallDeadline {
                mission_content_digest: document.identity.content_digest,
                expires_at_ns: start_ns + OPERATIONAL_WALL_DEADLINE_NS,
            },
        },
    )
    .expect("core starts");

    let arm = tick_core(&mut core, start_ns, observation(true, 500.0, false), None);
    let climb = tick_core(
        &mut core,
        start_ns,
        observation(true, 515.0, false),
        Some(succeeded_receipt(&arm)),
    );
    let follow = tick_core(
        &mut core,
        start_ns,
        observation(true, 515.0, false),
        Some(succeeded_receipt(&climb)),
    );
    tick_core(
        &mut core,
        start_ns,
        observation(true, 515.0, false),
        Some(succeeded_receipt(&follow)),
    );

    let boundary_ns = start_ns + FOLLOW_PLAN_PHASE_DEADLINE_NS - 1;
    let completed = tick_core(&mut core, boundary_ns, observation(true, 515.0, true), None);
    assert!(matches!(
        completed.state,
        EngineState::Terminal {
            result: MissionTerminal::Complete {
                completed_phases: 3
            }
        }
    ));
}

fn observation(guidance_valid: bool, altitude_m: f64, plan_complete: bool) -> MissionObservation {
    MissionObservation {
        navigation: NavigationObservation {
            guidance_valid: Some(guidance_valid),
            plan_complete: Some(plan_complete),
            altitude_m: Some(altitude_m),
        },
        vehicle: VehicleObservation::default(),
        signals: Vec::new(),
    }
}

fn succeeded_receipt(output: &pilotage_mission_core::TickOutput) -> DirectiveReceipt {
    let action_id = output
        .directives
        .first()
        .expect("phase emits a directive")
        .context()
        .action_id;
    DirectiveReceipt {
        action_id,
        result: ReceiptResult::Succeeded {},
    }
}

fn tick_core(
    engine: &mut CoreMissionEngine,
    now_ns: u64,
    observation: MissionObservation,
    receipt: Option<DirectiveReceipt>,
) -> pilotage_mission_core::TickOutput {
    engine
        .tick(TickInput {
            simulator_time_ns: now_ns,
            wall_time_ns: now_ns,
            observation,
            receipts: receipt.into_iter().collect(),
        })
        .expect("tick is accepted")
}
