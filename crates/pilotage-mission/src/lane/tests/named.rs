//! A kind added as a planner the lane registers by name: no wire message
//! of its own, planned, approved and run through the same lane.

use pilotage_mission_core::executor::{
    AT_MOST, ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

const HOST: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT, AT_MOST];

fn value(name: &str, value: &str) -> wire::MissionValue {
    wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

fn named(kind: &str, version: u32, parameters: Vec<wire::MissionValue>) -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "named-1".to_owned(),
        revision: 1,
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Named(wire::NamedObjective {
            kind: kind.to_owned(),
            version,
            parameters,
        })),
        ..wire::MissionObjective::default()
    }
}

fn lane() -> Lane {
    Lane::new(LaneConfig {
        executors: ExecutorRegistry::new(HOST),
        ..config()
    })
}

#[test]
fn a_recorded_value_is_read_and_reported_through_the_same_lane() {
    let mut lane = lane();
    let objective = named("record", 1, vec![value("source", "temperature")]);
    let reply = lane.on_intent(&intent(Action::Propose(objective)), &context("p"));
    let proposal = opened(&reply.receipt).expect("a proposal");
    let steps: Vec<String> = proposal
        .tasks
        .iter()
        .filter_map(|t| t.operation.as_ref().map(|o| o.operation.clone()))
        .collect();
    assert_eq!(steps, ["read-value", "write-report"]);
    assert!(accepted(&lane.on_intent(
        &intent(Action::Approve(approval(&proposal.digest, 0.0))),
        &context("a")
    )));
    let started = lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: proposal.digest,
        })),
        &context("run-1"),
    );
    assert!(matches!(started.effect, Some(LaneEffect::Start(_))));
    let ids: Vec<String> = proposal.tasks.iter().map(|t| t.task_id.clone()).collect();
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![
            (ids[0].clone(), vec![value("value", "21.5 C")]),
            (ids[1].clone(), vec![value("report", "run-1.txt")]),
        ],
        plan_complete: true,
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Complete);
}

#[test]
fn a_named_objective_the_lane_does_not_plan_as_given_is_refused() {
    let source = || vec![value("source", "temperature")];
    for (objective, why) in [
        (
            named("survey-of-mars", 1, source()),
            "no objective of the kind",
        ),
        (named("record", 2, source()), "version"),
        (named("record", 1, Vec::new()), "source"),
        (
            named(
                "record",
                1,
                vec![value("source", "t"), value("colour", "red")],
            ),
            "colour",
        ),
        (
            named(
                "record",
                1,
                vec![value("source", "a"), value("source", "b")],
            ),
            "twice",
        ),
        (
            named("record", 1, vec![value("", "temperature")]),
            "no name",
        ),
        (
            named("record", 1, vec![value("source", &"t".repeat(129))]),
            "source",
        ),
        (named("record", 1, vec![value("source", "")]), "no source"),
    ] {
        let mut lane = lane();
        let reply = lane.on_intent(&intent(Action::Propose(objective)), &context("p"));
        assert!(!accepted(&reply), "{why}");
        assert!(
            reply.receipt.reason.contains(why),
            "{why}: {}",
            reply.receipt.reason
        );
    }
}

#[test]
fn a_change_to_a_named_objective_is_a_change_of_objective() {
    use crate::change::ChangeClass;
    use crate::survey::objective_change::classify;
    let one = named("record", 1, vec![value("source", "temperature")]);
    assert!(classify(&one, &one.clone()).changes().is_empty());
    let other_source = named("record", 1, vec![value("source", "pressure")]);
    let other_kind = named("tally", 1, vec![value("source", "temperature")]);
    for after in [other_source, other_kind] {
        let changes = classify(&one, &after);
        assert!(changes.has(ChangeClass::Objective), "{changes:?}");
    }
}
