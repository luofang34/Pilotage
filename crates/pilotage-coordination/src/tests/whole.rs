//! A team passes an objective with no area to divide whole to one member
//! and runs it through its own lifecycle: the member's proposal, the
//! team's approval passed down, the start, the question the member's run
//! asks and the answer the team passes on, and the end.

use pilotage_mission::executor::{
    ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

const HOST: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT];

fn report() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "team-report".to_owned(),
        revision: 1,
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Report(
            wire::ReportObjective {
                source: "temperature".to_owned(),
                question: "Is this the cabin temperature?".to_owned(),
            },
        )),
        ..wire::MissionObjective::default()
    }
}

fn value(name: &str, value: &str) -> wire::MissionValue {
    wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

/// Reports `member`'s status to the team; returns the intents it sends.
fn report_status(fleet: &mut Fleet, member: &str) -> Vec<MemberIntent> {
    let status = fleet.members.get_mut(member).and_then(Lane::take_changed);
    status
        .map(|s| fleet.team.on_member_status(&s, &context("report")))
        .unwrap_or_default()
}

/// A team whose members' hosts carry out the report's operations, with the
/// report proposed, approved and started; returns it and the member that
/// took the report.
fn started() -> (Fleet, String) {
    let mut fleet = Fleet::new();
    for (name, lane) in &mut fleet.members {
        *lane = member_lane(name, true, ExecutorRegistry::new(HOST));
    }
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-propose".to_owned(),
            action: Some(Action::Propose(report())),
        },
        &context("team-proposal"),
    );
    assert_eq!(reply.intents.len(), 1, "one member takes it whole");
    let member = reply.intents[0].member.clone();
    let proposal = fleet.route(reply.intents)[0]
        .proposal_record
        .clone()
        .and_then(open_record)
        .expect("the team proposal");
    assert!(proposal.tasks.iter().all(|t| t.member == member));
    fleet.approve_and_start(approval(&proposal.digest, 0));
    (fleet, member)
}

/// `member`'s host read the value and asks the person about it; returns
/// the question and the member's task ids.
fn asks(fleet: &mut Fleet, member: &str) -> (wire::MissionQuestion, Vec<String>) {
    let lane = fleet.members.get_mut(member).expect("the member");
    let status = lane.status();
    let ids: Vec<String> = status.tasks.iter().map(|t| t.task_id.clone()).collect();
    let question = wire::MissionQuestion {
        operation_id: format!("{}/{}/1", status.run_id, ids[1]),
        prompt: "Is this the cabin temperature?".to_owned(),
        subject: "21.5 C".to_owned(),
        subject_digest: b"the digest of 21.5 C".to_vec(),
    };
    lane.on_progress(&RunProgress {
        active_task: Some(ids[1].clone()),
        finished_tasks: vec![ids[0].clone()],
        results: vec![(ids[0].clone(), vec![value("value", "21.5 C")])],
        questions: vec![(ids[1].clone(), question.clone())],
        ..RunProgress::default()
    });
    assert!(report_status(fleet, member).is_empty());
    (question, ids)
}

#[test]
fn a_team_runs_a_report_through_one_member_and_passes_the_answer_on() {
    let (mut fleet, member) = started();
    let (question, ids) = asks(&mut fleet, &member);
    let asked: Vec<_> = fleet
        .team
        .status()
        .tasks
        .into_iter()
        .filter_map(|t| t.question)
        .collect();
    assert_eq!(
        asked,
        std::slice::from_ref(&question),
        "the team shows the member's question"
    );
    // The operator answers the team; the team passes it to the member.
    let answer = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-answer".to_owned(),
            action: Some(Action::Answer(wire::MissionAnswer {
                operation_id: question.operation_id.clone(),
                subject_digest: question.subject_digest.clone(),
                accepted: true,
                answered_by: None,
            })),
        },
        &context("answer"),
    );
    assert!(answer.receipt.is_none(), "it waits for the member");
    assert_eq!(answer.intents.len(), 1);
    assert_eq!(answer.intents[0].member, member);
    let decided = fleet.route(answer.intents);
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(decided[0].request_id, "op-answer");
    assert!(accepted(&decided[0]), "{}", decided[0].reason);
    // The member's host wrote the report: the team ends with its member.
    let lane = fleet.members.get_mut(&member).expect("the member");
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![
            (
                ids[1].clone(),
                vec![value("by", "operator"), value("confirmed", "true")],
            ),
            (ids[2].clone(), vec![value("report", "r.txt")]),
        ],
        plan_complete: true,
        ..RunProgress::default()
    });
    report_status(&mut fleet, &member);
    let team = fleet.team.status();
    assert_eq!(
        team.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        team.detail
    );
    assert!(
        team.tasks
            .iter()
            .all(|t| t.judgment == wire::MissionTaskJudgment::Met as i32)
    );
}

#[test]
fn an_answer_no_member_asks_for_is_refused_by_the_team() {
    let mut fleet = Fleet::new();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-answer".to_owned(),
            action: Some(Action::Answer(wire::MissionAnswer {
                operation_id: "run-1/x/1".to_owned(),
                ..wire::MissionAnswer::default()
            })),
        },
        &context("answer"),
    );
    let receipt = reply.receipt.expect("answered at once");
    assert!(
        !accepted(&receipt) && reply.intents.is_empty(),
        "{}",
        receipt.reason
    );
}
