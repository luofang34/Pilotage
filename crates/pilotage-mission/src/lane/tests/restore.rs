//! A lane rebuilt from its record after the host restarted replays
//! nothing: it asks what happened, runs again only what is safe to repeat
//! under the same identity, and leaves the rest to a person.

use pilotage_mission_core::executor::{
    ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionTaskExecution as Execution;
use wire::mission_intent::Action;

use super::report::{answer, asking, started, task, value};
use super::*;
use crate::lane::{Reconciled, operation_id};

/// A host whose read-value cannot tell after a restart whether it happened,
/// and whose repeat is not safe.
const UNTOLD: &[OperationDescriptor] = &[
    OperationDescriptor {
        idempotent: false,
        status_query: false,
        ..READ_VALUE
    },
    WAIT_FOR_PERSON,
    WRITE_REPORT,
];

/// A host whose report can neither tell after a restart whether it was
/// written nor be repeated safely.
const UNTOLD_REPORT: &[OperationDescriptor] = &[
    READ_VALUE,
    WAIT_FOR_PERSON,
    OperationDescriptor {
        idempotent: false,
        status_query: false,
        ..WRITE_REPORT
    },
];

fn report_config(executors: &'static [OperationDescriptor]) -> LaneConfig {
    LaneConfig {
        executors: ExecutorRegistry::new(executors),
        ..config()
    }
}

/// The lane `lane`'s record rebuilds, under `executors`.
fn restarted(lane: &Lane, executors: &'static [OperationDescriptor]) -> Lane {
    let record = lane.record().expect("a run to record");
    Lane::restore(report_config(executors), &record).expect("restored")
}

fn execution(lane: &Lane, id: &str) -> Execution {
    Execution::try_from(task(lane, id).execution).expect("an execution")
}

fn queried(lane: &Lane) -> Vec<String> {
    lane.status_queries()
        .into_iter()
        .map(|q| q.task_id)
        .collect()
}

#[test]
fn an_idle_lane_has_nothing_to_record() {
    assert_eq!(Lane::new(config()).record(), None);
}

#[test]
fn a_restarted_run_asks_what_happened_and_waits_for_a_person() {
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    let mut lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(lane.state(), State::Uncertain);
    // The operation in progress is asked about under its own identity; the
    // report after it never began.
    assert_eq!(queried(&lane), [ids[1].clone()]);
    let identities: Vec<String> = lane
        .status_queries()
        .into_iter()
        .map(|q| q.operation_id)
        .collect();
    assert_eq!(identities, [operation_id("run-1", &ids[1])]);
    assert_eq!(execution(&lane, &ids[0]), Execution::Completed);
    assert_eq!(execution(&lane, &ids[2]), Execution::Pending);
    let early = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert!(!accepted(&early), "nothing resumes while it is uncertain");

    lane.reconcile(&ids[1], Reconciled::DidNotHappen)
        .expect("asked");
    assert_eq!(lane.state(), State::Held);
    assert_eq!(
        lane.status().holds,
        [wire::MissionHoldReason::Restarted as i32]
    );
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    let Some(LaneEffect::Start(order)) = resumed.effect else {
        panic!("what is left starts: {}", resumed.receipt.reason);
    };
    assert_eq!(order.run_id, "run-1", "the run keeps its identity");
    let RunPlan::Operations(left) = order.plan else {
        panic!("a plan of operations");
    };
    assert_eq!(left.len(), 2, "the person's step and the report");
    assert_eq!(lane.state(), State::Running);
}

#[test]
fn what_happened_before_the_restart_is_not_done_again() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    assert!(accepted(
        &lane.on_intent(&answer(&question, true), &context("yes"))
    ));
    let mut lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(execution(&lane, &ids[0]), Execution::Completed);
    assert_eq!(queried(&lane), [ids[1].clone()]);
    let confirmed = vec![value("by", "operator"), value("confirmed", "true")];
    lane.reconcile(&ids[1], Reconciled::Happened(confirmed.clone()))
        .expect("asked");
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    let Some(LaneEffect::Start(order)) = resumed.effect else {
        panic!("what is left starts: {}", resumed.receipt.reason);
    };
    let RunPlan::Operations(left) = order.plan else {
        panic!("a plan of operations");
    };
    let left: Vec<String> = left.into_iter().map(|p| p.task_id).collect();
    assert_eq!(left, [ids[2].clone()], "only the report is left");
    assert_eq!(
        order.results,
        [
            (ids[0].clone(), vec![value("value", "21.5 C")]),
            (ids[1].clone(), confirmed),
        ],
        "the report takes what the run had"
    );
}

#[test]
fn a_run_whose_every_operation_happened_completes() {
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    // The person's step ended, and the report is in progress.
    lane.on_progress(&RunProgress {
        active_task: Some(ids[2].clone()),
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        results: vec![(
            ids[1].clone(),
            vec![value("by", "operator"), value("confirmed", "true")],
        )],
        ..RunProgress::default()
    });
    let mut lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(queried(&lane), [ids[2].clone()]);
    lane.reconcile(
        &ids[2],
        Reconciled::Happened(vec![value("report", "run-1.txt")]),
    )
    .expect("asked");
    assert_eq!(lane.state(), State::Complete);
    assert!(lane.reconcile(&ids[2], Reconciled::DidNotHappen).is_err());
}

#[test]
fn an_operation_nobody_can_tell_about_stays_uncertain_until_an_abort() {
    let (mut lane, ids) = started();
    lane.on_progress(&RunProgress {
        active_task: Some(ids[0].clone()),
        ..RunProgress::default()
    });
    let mut lane = restarted(&lane, UNTOLD);
    assert_eq!(execution(&lane, &ids[0]), Execution::Uncertain);
    assert!(queried(&lane).is_empty(), "nothing else may have begun");
    assert_eq!(lane.state(), State::Uncertain, "{}", lane.status().detail);
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert!(!accepted(&resumed));
    let aborted = lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    assert!(matches!(aborted.effect, Some(LaneEffect::Abort)));
    assert_eq!(lane.state(), State::Aborted);
    // An abort does not show that the operation had no effect.
    assert_eq!(execution(&lane, &ids[0]), Execution::Uncertain);
}

#[test]
fn a_cancelled_run_is_not_replayed() {
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    assert!(accepted(&lane.on_intent(
        &intent(Action::Abort(wire::MissionAbort {})),
        &context("x")
    )));
    lane.on_progress(&RunProgress {
        cancelled_tasks: vec![ids[1].clone()],
        ended: true,
        ..RunProgress::default()
    });
    let mut lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(lane.state(), State::Aborted);
    assert_eq!(execution(&lane, &ids[1]), Execution::Cancelled);
    assert_eq!(execution(&lane, &ids[2]), Execution::Pending);
    assert!(lane.status_queries().is_empty());
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert!(!accepted(&resumed));
    assert_eq!(resumed.effect, None);
}

#[test]
fn a_restarted_flight_is_left_to_a_person() {
    let (lane, _) = running_lane();
    let record = lane.record().expect("a run to record");
    let mut lane = Lane::restore(config(), &record).expect("restored");
    assert_eq!(lane.state(), State::Uncertain);
    assert!(lane.status().detail.contains("check the vehicle"));
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert!(!accepted(&resumed));
    let aborted = lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    assert!(matches!(aborted.effect, Some(LaneEffect::Abort)));
}

#[test]
fn a_record_that_does_not_open_is_refused() {
    let (lane, _) = started();
    let mut record = lane.record().expect("a run to record");
    if let Some(proposal) = record.proposal.as_mut() {
        proposal.body.push(0);
    }
    assert!(Lane::restore(report_config(&[READ_VALUE]), &record).is_err());
}

#[test]
fn a_run_after_an_aborted_restart_resumes_as_any_run() {
    let (lane, _) = running_lane();
    let record = lane.record().expect("a run to record");
    let mut lane = Lane::restore(
        report_config(&[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]),
        &record,
    )
    .expect("restored");
    assert!(accepted(&lane.on_intent(
        &intent(Action::Abort(wire::MissionAbort {})),
        &context("x")
    )));
    // A new report runs on the same lane, holds and resumes.
    let reply = lane.on_intent(
        &intent(Action::Propose(super::report::report())),
        &context("p2"),
    );
    let proposal = opened(&reply.receipt).expect("a proposal");
    assert!(accepted(&lane.on_intent(
        &intent(Action::Approve(approval(&proposal.digest, 0.0))),
        &context("a2")
    )));
    let start = lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: proposal.digest,
        })),
        &context("run-2"),
    );
    assert!(matches!(start.effect, Some(LaneEffect::Start(_))));
    assert!(accepted(&lane.on_intent(
        &intent(Action::Hold(wire::MissionHold {})),
        &context("h")
    )));
    lane.on_progress(&RunProgress {
        holding: true,
        ..RunProgress::default()
    });
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert_eq!(
        resumed.effect,
        Some(LaneEffect::Resume),
        "{}",
        resumed.receipt.reason
    );
}

#[test]
fn an_operation_an_abort_overtook_is_asked_about() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    assert!(accepted(
        &lane.on_intent(&answer(&question, true), &context("yes"))
    ));
    // The person's step ended; the report may have begun when the abort came.
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        results: vec![(
            ids[1].clone(),
            vec![value("by", "operator"), value("confirmed", "true")],
        )],
        ..RunProgress::default()
    });
    assert!(accepted(&lane.on_intent(
        &intent(Action::Abort(wire::MissionAbort {})),
        &context("x")
    )));
    let mut lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(lane.state(), State::Aborted);
    assert_eq!(queried(&lane), [ids[2].clone()]);
    lane.reconcile(
        &ids[2],
        Reconciled::Happened(vec![value("report", "run-1.txt")]),
    )
    .expect("asked");
    assert_eq!(execution(&lane, &ids[2]), Execution::Completed);
    assert_eq!(lane.state(), State::Aborted, "the run stays as it ended");
}

#[test]
fn operations_after_the_one_in_progress_never_began() {
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    let mut lane = restarted(&lane, UNTOLD_REPORT);
    // Only the person's step may have begun; the report never did.
    assert_eq!(queried(&lane), [ids[1].clone()]);
    assert_eq!(execution(&lane, &ids[2]), Execution::Pending);
    lane.reconcile(&ids[1], Reconciled::DidNotHappen)
        .expect("asked");
    assert_eq!(lane.state(), State::Held, "{}", lane.status().detail);
}

#[test]
fn a_second_restart_keeps_the_uncertain_operation_and_nothing_after_it_began() {
    let (mut lane, ids) = started();
    asking(&mut lane, &ids);
    // The host could not answer the query about the person's step.
    let first = restarted(&lane, UNTOLD_REPORT);
    assert_eq!(queried(&first), [ids[1].clone()]);
    let second = restarted(&first, UNTOLD_REPORT);
    assert_eq!(queried(&second), [ids[1].clone()]);
    assert_eq!(execution(&second, &ids[2]), Execution::Pending);
}

#[test]
fn a_run_that_ended_without_an_abort_began_nothing_more() {
    let (mut lane, ids) = started();
    let question = asking(&mut lane, &ids);
    assert!(accepted(
        &lane.on_intent(&answer(&question, false), &context("no"))
    ));
    // The person declines: the run ends there, between two operations.
    lane.on_progress(&RunProgress {
        finished_tasks: vec![ids[0].clone(), ids[1].clone()],
        results: vec![(
            ids[1].clone(),
            vec![value("by", "operator"), value("confirmed", "false")],
        )],
        ended: true,
        ..RunProgress::default()
    });
    let ended = lane.state();
    let lane = restarted(&lane, &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT]);
    assert_eq!(lane.state(), ended);
    assert!(lane.status_queries().is_empty());
    assert_eq!(execution(&lane, &ids[2]), Execution::Pending);
}
