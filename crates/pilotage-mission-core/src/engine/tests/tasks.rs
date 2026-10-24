//! Tasks as execution units: one engine sequences survey segments of one plan and a task
//! that is no survey at all, in the order their dependencies allow, with no survey
//! branch in the core.

use std::collections::BTreeMap;

use crate::{
    BoundAction, CodecError, Digest, EngineEvent, ExecutionPolicy, ExecutionTarget, FlightAction,
    FlightPlanReference, MissionAction, MissionCapability, MissionCondition, MissionDirective,
    MissionDocument, MissionObservation, MissionPhase, MissionTerminal, NavigationCondition,
    NavigationDataIdentity, NavigationObservation, ValidationError,
};

use super::support::{digest, document, engine, phase, succeeded, terminal, tick};

fn plan() -> FlightPlanReference {
    FlightPlanReference {
        plan_id: "survey".to_owned(),
        plan_content_digest: Digest::from_bytes([9; 32]),
        navigation_data_identity: NavigationDataIdentity {
            cycle: "2608".to_owned(),
            snapshot_id: "nav-engine-1".to_owned(),
            snapshot_digest: digest(7),
        },
    }
}

/// A segment phase that flies toward waypoint `waypoint` and ends once it is captured.
fn segment(id: &str, waypoint: u32) -> MissionPhase {
    MissionPhase {
        required_capabilities: vec![
            MissionCapability::FlightPlan,
            MissionCapability::NavigationState,
        ],
        action: MissionAction::Flight(FlightAction::FollowSegment {
            plan: plan(),
            first_waypoint: waypoint,
            last_waypoint: waypoint,
        }),
        completion_conditions: vec![MissionCondition::Navigation(
            NavigationCondition::WaypointCaptured { index: waypoint },
        )],
        deadline: None,
        ..phase(id)
    }
}

/// A task for a person: the host's `person` executor confirms it.
fn ask_a_person(id: &str, depends_on: &[&str]) -> MissionPhase {
    MissionPhase {
        required_capabilities: Vec::new(),
        action: MissionAction::Bound(BoundAction {
            executor: "person".to_owned(),
            operation: "confirm".to_owned(),
            parameters: BTreeMap::from([("question".to_owned(), "is the area clear".to_owned())]),
        }),
        completion_conditions: vec![MissionCondition::Always {}],
        depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
        deadline: None,
        ..phase(id)
    }
}

fn captured(count: u32) -> MissionObservation {
    MissionObservation {
        navigation: NavigationObservation {
            waypoints_captured: Some(count),
            ..NavigationObservation::default()
        },
        ..MissionObservation::default()
    }
}

fn entered(events: &[EngineEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::PhaseEntered { phase_id, .. } => Some(phase_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn one_core_runs_survey_segments_and_a_task_for_a_person_in_dependency_order() {
    // Listed first, the person's task waits for the first line.
    let mut engine = engine(document(vec![
        ask_a_person("confirm-clear", &["line-01"]),
        segment("line-01", 0),
        segment("line-02", 1),
    ]));
    let mut order = Vec::new();
    let mut flown = 0;
    let mut output = tick(&mut engine, 0, 0, captured(flown), Vec::new());
    order.extend(entered(&output.events));
    for time in 1..20 {
        if matches!(output.state, crate::EngineState::Terminal { .. }) {
            break;
        }
        // The vehicle flies the segment it was told; a person answers the question.
        match output.directives.first() {
            Some(MissionDirective::Flight(directive)) => {
                if let FlightAction::FollowSegment { last_waypoint, .. } = directive.action {
                    flown = last_waypoint + 1;
                }
            }
            Some(MissionDirective::Bound(directive)) => {
                assert_eq!(directive.action.executor, "person");
            }
            _ => {}
        }
        let receipts = output
            .directives
            .first()
            .map(|_| succeeded(&output))
            .into_iter()
            .collect();
        output = tick(&mut engine, time, time, captured(flown), receipts);
        order.extend(entered(&output.events));
    }
    assert_eq!(order, ["line-01", "confirm-clear", "line-02"]);
    assert!(matches!(
        terminal(&output),
        MissionTerminal::Complete {
            completed_phases: 3
        }
    ));
}

#[test]
fn a_dependency_on_a_missing_phase_itself_or_a_cycle_is_refused() {
    let refused = |phases: Vec<MissionPhase>| {
        matches!(
            MissionDocument::new(
                "dependencies".to_owned(),
                None,
                ExecutionPolicy {
                    target: ExecutionTarget::Simulator,
                    retry_limit: 0,
                    receipt_timeout_ns: 1,
                },
                phases,
            ),
            Err(CodecError::Validation(
                ValidationError::InvalidDependency { .. }
            ))
        )
    };
    assert!(refused(vec![ask_a_person("a", &["nowhere"])]));
    assert!(refused(vec![ask_a_person("a", &["a"])]));
    assert!(refused(vec![
        ask_a_person("a", &["b"]),
        ask_a_person("b", &["a"])
    ]));
    assert!(!refused(vec![
        ask_a_person("a", &[]),
        ask_a_person("b", &["a"])
    ]));
}
