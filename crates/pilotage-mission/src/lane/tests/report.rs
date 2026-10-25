//! A task kind that is not a survey goes through the same lane: a report
//! objective reads a value, asks a person to confirm it and writes a
//! report, each step an operation of a host executor. Its proposal, its
//! approval, its run, its answer and its judgment are the lane's own.

use pilotage_mission_core::executor::{
    ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use sha2::{Digest, Sha256};
use wire::MissionTaskExecution as Execution;
use wire::MissionTaskJudgment as Judgment;
use wire::mission_intent::Action;

use super::*;

const HOST: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT];
const VALUE: &str = "21.5 C";

fn report() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "report-1".to_owned(),
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

/// A lane whose host carries out the report's operations, with its report
/// proposed, approved and started; returns it and the task ids in order.
fn started() -> (Lane, Vec<String>) {
    let mut lane = Lane::new(LaneConfig {
        executors: ExecutorRegistry::new(HOST),
        ..config()
    });
    let reply = lane.on_intent(&intent(Action::Propose(report())), &context("p"));
    let proposal = opened(&reply.receipt).expect("a report proposal");
    assert!(
        proposal
            .requires
            .contains(&"pilotage.task.operation/1".to_owned()),
        "{:?}",
        proposal.requires
    );
    // A plan that flies nowhere is approved to climb to no height.
    let approved = lane.on_intent(
        &intent(Action::Approve(approval(&proposal.digest, 0.0))),
        &context("a"),
    );
    assert!(accepted(&approved), "{}", approved.receipt.reason);
    let started = lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: proposal.digest,
        })),
        &context("run-1"),
    );
    let Some(LaneEffect::Start(order)) = started.effect else {
        panic!("a run: {}", started.receipt.reason);
    };
    let RunPlan::Operations(plan) = order.plan else {
        panic!("a plan of operations");
    };
    let ids = plan.iter().map(|step| step.task_id.clone()).collect();
    (lane, ids)
}

fn value(name: &str, value: &str) -> wire::MissionValue {
    wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

/// The run read the value and asks the person about it.
fn asking(lane: &mut Lane, ids: &[String]) -> wire::MissionQuestion {
    let question = wire::MissionQuestion {
        operation_id: format!("run-1/{}/1", ids[1]),
        prompt: "Is this the cabin temperature?".to_owned(),
        subject: VALUE.to_owned(),
        subject_digest: Sha256::digest(VALUE.as_bytes()).to_vec(),
    };
    lane.on_progress(&RunProgress {
        active_task: Some(ids[1].clone()),
        finished_tasks: vec![ids[0].clone()],
        results: vec![(ids[0].clone(), vec![value("value", VALUE)])],
        questions: vec![(ids[1].clone(), question.clone())],
        ..RunProgress::default()
    });
    question
}

fn answer(question: &wire::MissionQuestion, accepted: bool) -> wire::MissionIntent {
    intent(Action::Answer(wire::MissionAnswer {
        operation_id: question.operation_id.clone(),
        subject_digest: question.subject_digest.clone(),
        accepted,
        answered_by: None,
    }))
}

fn task(lane: &Lane, id: &str) -> wire::MissionTaskStatus {
    lane.status()
        .tasks
        .into_iter()
        .find(|t| t.task_id == id)
        .expect("the task")
}

#[test]
fn a_confirmed_report_completes_with_every_step_met() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    assert_eq!(
        task(&lane, &ids[1]).question,
        Some(question.clone()),
        "the status asks"
    );
    assert_eq!(task(&lane, &ids[0]).results, [value("value", VALUE)]);
    let reply = lane.on_intent(&answer(&question, true), &context("yes"));
    assert!(accepted(&reply), "{}", reply.receipt.reason);
    let Some(LaneEffect::Answer(given)) = reply.effect else {
        panic!("the answer goes to the operation");
    };
    assert!(given.accepted);
    assert_eq!(given.operation_id, question.operation_id);
    let again = lane.on_intent(&answer(&question, true), &context("again"));
    assert!(!accepted(&again), "an answered question is asked no more");
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![
            (ids[0].clone(), vec![value("value", VALUE)]),
            (
                ids[1].clone(),
                vec![value("by", "operator"), value("confirmed", "true")],
            ),
            (ids[2].clone(), vec![value("report", "run-1_write_1.txt")]),
        ],
        plan_complete: true,
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Complete);
    for id in &ids {
        let status = task(&lane, id);
        assert_eq!(status.execution, Execution::Completed as i32, "{id}");
        assert_eq!(status.judgment, Judgment::Met as i32, "{id}");
    }
}

#[test]
fn an_answer_to_another_subject_is_refused_and_the_question_stays() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    let mut stale = question.clone();
    stale.subject_digest = Sha256::digest(b"20.0 C").to_vec();
    let reply = lane.on_intent(&answer(&stale, true), &context("stale"));
    assert!(!accepted(&reply), "the person saw another value");
    assert!(reply.effect.is_none());
    assert_eq!(task(&lane, &ids[1]).question, Some(question));
}

#[test]
fn a_declined_value_is_not_met_and_the_report_is_not_written() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    lane.on_intent(&answer(&question, false), &context("no"));
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        results: vec![(
            ids[1].clone(),
            vec![value("by", "operator"), value("confirmed", "false")],
        )],
        ended: true,
        ended_because: Some("the person declined"),
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Failed);
    assert_eq!(task(&lane, &ids[1]).judgment, Judgment::NotMet as i32);
    assert_eq!(
        task(&lane, &ids[2]).execution,
        Execution::Pending as i32,
        "never begun"
    );
}

#[test]
fn an_abort_while_the_report_is_written_waits_for_how_the_write_ended() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    lane.on_intent(&answer(&question, true), &context("yes"));
    lane.on_progress(&RunProgress {
        active_task: Some(ids[2].clone()),
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        ..RunProgress::default()
    });
    let aborted = lane.on_intent(
        &intent(Action::Abort(wire::MissionAbort {})),
        &context("abort"),
    );
    assert!(accepted(&aborted));
    assert_eq!(
        task(&lane, &ids[2]).execution,
        Execution::Active as i32,
        "a write runs to its end: it is not taken as cancelled"
    );
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![(ids[2].clone(), vec![value("report", "run-1_write_1.txt")])],
        ..RunProgress::default()
    });
    assert_eq!(task(&lane, &ids[2]).execution, Execution::Completed as i32);
    assert_eq!(lane.state(), wire::MissionLaneState::Aborted);
}

#[test]
fn a_host_without_the_operations_refuses_the_report_when_it_is_proposed() {
    let mut lane = Lane::new(LaneConfig {
        executors: ExecutorRegistry::new(&[WAIT_FOR_PERSON]),
        ..config()
    });
    let reply = lane.on_intent(&intent(Action::Propose(report())), &context("p"));
    assert!(!accepted(&reply));
    assert!(
        reply.receipt.reason.contains("host/read-value"),
        "{}",
        reply.receipt.reason
    );
    assert_eq!(lane.state(), wire::MissionLaneState::Idle);
}

#[test]
fn only_an_approver_answers_and_the_host_writes_who_did() {
    let question = wire::MissionQuestion {
        operation_id: "run-1/confirm/1".to_owned(),
        ..wire::MissionQuestion::default()
    };
    let steering = crate::lane::Steering::default();
    let viewer = crate::lane::Requester {
        subject: "viewer".to_owned(),
        may_approve: false,
    };
    let mut intent = answer(&question, true);
    assert!(steering.admit(&mut intent, &viewer).is_err());
    let approver = crate::lane::Requester {
        subject: "operator".to_owned(),
        may_approve: true,
    };
    steering.admit(&mut intent, &approver).expect("admitted");
    let Some(Action::Answer(given)) = intent.action else {
        panic!("an answer");
    };
    assert_eq!(
        given.answered_by.map(|o| o.subject).as_deref(),
        Some("operator")
    );
}

#[test]
fn an_answered_question_the_host_reports_again_is_asked_no_more() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    let yes = lane.on_intent(&answer(&question, true), &context("yes"));
    assert!(accepted(&yes));
    // The runner has not taken the answer yet and reports the question again.
    asking(&mut lane, &ids);
    assert_eq!(task(&lane, &ids[1]).question, None);
    let no = lane.on_intent(&answer(&question, false), &context("no"));
    assert!(!accepted(&no), "one answer per question");
    assert!(no.effect.is_none());
}

#[test]
fn an_abort_leaves_each_operation_to_the_end_the_host_reports() {
    let abort = |lane: &mut Lane| {
        let reply = lane.on_intent(
            &intent(Action::Abort(wire::MissionAbort {})),
            &context("abort"),
        );
        assert!(accepted(&reply));
    };
    // The host stopped the question: the task is cancelled once it says so.
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    abort(&mut lane);
    assert_eq!(
        task(&lane, &ids[1]).execution,
        Execution::Active as i32,
        "not known yet"
    );
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone()],
        cancelled_tasks: vec![ids[1].clone()],
        ..RunProgress::default()
    });
    assert_eq!(task(&lane, &ids[1]).execution, Execution::Cancelled as i32);
    // The host took the answer before the abort: the task completed.
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    lane.on_intent(&answer(&question, true), &context("yes"));
    abort(&mut lane);
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        results: vec![(
            ids[1].clone(),
            vec![value("by", "operator"), value("confirmed", "true")],
        )],
        ..RunProgress::default()
    });
    let confirm = task(&lane, &ids[1]);
    assert_eq!(confirm.execution, Execution::Completed as i32);
    assert_eq!(confirm.judgment, Judgment::Met as i32);
    // A write that fails after the abort is failed, not left in progress.
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        failed_tasks: vec![ids[2].clone()],
        ..RunProgress::default()
    });
    assert_eq!(task(&lane, &ids[2]).execution, Execution::Failed as i32);
}

#[test]
fn an_expired_approval_holds_a_plan_of_operations_and_flies_nothing() {
    let mut lane = Lane::new(LaneConfig {
        executors: ExecutorRegistry::new(HOST),
        ..config()
    });
    let reply = lane.on_intent(&intent(Action::Propose(report())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    let mut home_on_expiry = approval(&digest, 0.0);
    if let Some(envelope) = home_on_expiry.envelope.as_mut() {
        envelope.on_expiry = wire::MissionExpiryPolicy::ReturnHome as i32;
    }
    lane.on_intent(&intent(Action::Approve(home_on_expiry)), &context("a"));
    lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: digest,
        })),
        &context("run-1"),
    );
    let effect = lane.on_clock(NOW + 10 * 3_600_000);
    assert_eq!(effect, Some(LaneEffect::Hold), "no way home to fly");
    assert!(lane.status().successor.is_none());
}

#[test]
fn a_plan_of_operations_takes_no_revision() {
    let (mut lane, _) = started();
    let status = lane.status();
    let reply = lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            change: Some(wire::mission_revise::Change::Finish(wire::MissionFinish {})),
            ..wire::MissionRevise::default()
        })),
        &context("rev"),
    );
    assert!(!accepted(&reply));
    assert!(
        reply.receipt.reason.contains("no revision"),
        "{}",
        reply.receipt.reason
    );
}

#[test]
fn a_run_that_ends_without_reporting_its_operation_leaves_it_unknown() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    lane.on_intent(&answer(&question, true), &context("yes"));
    lane.on_progress(&RunProgress {
        active_task: Some(ids[2].clone()),
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        ..RunProgress::default()
    });
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        ended: true,
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Failed);
    assert_eq!(
        task(&lane, &ids[2]).execution,
        Execution::Active as i32,
        "the write may have happened; nothing reported how it ended"
    );
}

#[test]
fn an_approver_that_relays_an_answer_keeps_who_answered() {
    let steering = crate::lane::Steering::default();
    let team = crate::lane::Requester {
        subject: "pilotage-coordinator".to_owned(),
        may_approve: true,
    };
    let mut relayed = intent(Action::Answer(wire::MissionAnswer {
        operation_id: "run-1/confirm/1".to_owned(),
        answered_by: Some(wire::RecordOrigin {
            issuer: crate::lane::HOST_ISSUER.to_owned(),
            subject: "operator".to_owned(),
            organization: String::new(),
        }),
        ..wire::MissionAnswer::default()
    }));
    steering.admit(&mut relayed, &team).expect("admitted");
    let Some(Action::Answer(given)) = relayed.action else {
        panic!("an answer");
    };
    assert_eq!(
        given.answered_by.map(|o| o.subject).as_deref(),
        Some("operator")
    );
}
