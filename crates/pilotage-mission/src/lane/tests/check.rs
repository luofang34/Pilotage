//! A second kind made of operations, decided by a rule: the lane plans,
//! runs and judges it through the same contracts as the report kind.

use pilotage_mission_core::executor::{
    AT_MOST, ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

const HOST: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT, AT_MOST];

fn check() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "check-1".to_owned(),
        revision: 1,
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Check(wire::CheckObjective {
            source: "temperature".to_owned(),
            limit: "30".to_owned(),
        })),
        ..wire::MissionObjective::default()
    }
}

fn lane_with(executors: &'static [OperationDescriptor]) -> Lane {
    Lane::new(LaneConfig {
        executors: ExecutorRegistry::new(executors),
        ..config()
    })
}

#[test]
fn a_check_reads_holds_to_its_limit_and_reports_the_verdict() {
    let mut lane = lane_with(HOST);
    let reply = lane.on_intent(&intent(Action::Propose(check())), &context("p"));
    let proposal = opened(&reply.receipt).expect("a check proposal");
    let steps: Vec<(String, String)> = proposal
        .tasks
        .iter()
        .filter_map(|t| {
            let op = t.operation.as_ref()?;
            Some((op.executor.clone(), op.operation.clone()))
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("host".to_owned(), "read-value".to_owned()),
            ("rule".to_owned(), "at-most".to_owned()),
            ("host".to_owned(), "write-report".to_owned()),
        ]
    );
    // The report takes the rule's verdict, which takes the value read.
    let takes = |task: &wire::MissionTaskPlan, name: &str| {
        task.operation
            .as_ref()
            .and_then(|o| o.parameters.iter().find(|p| p.name == name))
            .and_then(|p| match &p.source {
                Some(wire::mission_parameter::Source::ResultOf(of)) => {
                    Some((of.task_id.clone(), of.result.clone()))
                }
                _ => None,
            })
    };
    let ids: Vec<String> = proposal.tasks.iter().map(|t| t.task_id.clone()).collect();
    assert_eq!(
        takes(&proposal.tasks[1], "value"),
        Some((ids[0].clone(), "value".to_owned()))
    );
    assert_eq!(
        takes(&proposal.tasks[2], "value"),
        Some((ids[1].clone(), "verdict".to_owned()))
    );

    // It is approved and run as any plan of operations.
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
    let value = |name: &str, value: &str| wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    };
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![
            (ids[0].clone(), vec![value("value", "31.2 C")]),
            (
                ids[1].clone(),
                vec![
                    value("verdict", "31.2 C is above the limit of 30"),
                    value("within", "false"),
                ],
            ),
            (ids[2].clone(), vec![value("report", "run-1.txt")]),
        ],
        plan_complete: true,
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Complete);
}

#[test]
fn a_host_without_the_rule_refuses_a_check_before_it_runs() {
    let mut lane = lane_with(&[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    let reply = lane.on_intent(&intent(Action::Propose(check())), &context("p"));
    assert!(!accepted(&reply));
    assert!(
        reply.receipt.reason.contains("rule/at-most"),
        "{}",
        reply.receipt.reason
    );
}

#[test]
fn a_check_whose_limit_is_no_number_is_refused_when_proposed() {
    let mut lane = lane_with(HOST);
    let mut objective = check();
    if let Some(wire::mission_objective::Kind::Check(check)) = objective.kind.as_mut() {
        check.limit = "thirty".to_owned();
    }
    let reply = lane.on_intent(&intent(Action::Propose(objective)), &context("p"));
    assert!(!accepted(&reply));
    assert!(
        reply.receipt.reason.contains("not a number"),
        "{}",
        reply.receipt.reason
    );
}
