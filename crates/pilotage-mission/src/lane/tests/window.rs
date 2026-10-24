//! A task whose execution ended still takes the answers to its pictures for
//! its evidence window, also after a successor replaced its run. The window
//! runs on the host's monotonic clock, which does not stop while the run
//! holds; an answer after it closes is counted as late and changes nothing.

use pilotage_protocol::wire;
use wire::MissionTaskJudgment as Judgment;
use wire::mission_intent::Action;

use super::*;
use crate::lane::{FrameTicket, VerdictRefused};

fn usable() -> Option<FrameAssessment> {
    Some(FrameAssessment {
        usable: true,
        hazard: false,
    })
}

/// Two pictures of the line in progress, sent for judgment.
fn sent_on_line(lane: &mut Lane, line: usize) -> Vec<FrameTicket> {
    lane.on_progress(&toward(lane, 2 * line - 1));
    let tickets: Vec<FrameTicket> = (0..2).filter_map(|_| lane.on_frame()).collect();
    assert_eq!(tickets.len(), 2);
    for ticket in &tickets {
        lane.on_sent(ticket);
    }
    tickets
}

/// Drafts `change`, approves it and flies to the end of the line in
/// progress, where the lane switches to the successor.
fn switch_after_line(lane: &mut Lane, line: usize, change: wire::mission_revise::Change) {
    let status = lane.status();
    let drafted = lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            within_envelope: false,
            drafted_by: None,
            release: None,
            share_of: String::new(),
            change: Some(change),
        })),
        &context(&format!("rev-{}", status.run_id)),
    );
    let draft = opened(&drafted.receipt).expect("a draft");
    let approved = lane.on_intent(
        &intent(Action::Approve(approval(&draft.digest, 120.0))),
        &context("approve"),
    );
    assert!(accepted(&approved), "{}", approved.receipt.reason);
    let switched = lane.on_progress(&toward(lane, 2 * line + 1));
    assert!(matches!(switched, Some(LaneEffect::Start(_))));
}

fn finish() -> wire::mission_revise::Change {
    wire::mission_revise::Change::Finish(wire::MissionFinish {})
}

/// The tasks named `name`, oldest run first.
fn tasks_named(lane: &Lane, name: &str) -> Vec<wire::MissionTaskStatus> {
    lane.status()
        .tasks
        .into_iter()
        .filter(|t| t.name == name)
        .collect()
}

fn collection(task: &wire::MissionTaskStatus) -> wire::MissionEvidenceCollection {
    task.evidence
        .as_ref()
        .and_then(|e| wire::MissionEvidenceCollection::try_from(e.collection).ok())
        .unwrap_or_default()
}

#[test]
fn a_replaced_run_takes_the_answers_due_inside_the_window() {
    let (mut lane, _) = running_lane();
    lane.on_host_clock(1_000);
    let tickets = sent_on_line(&mut lane, 1);
    switch_after_line(&mut lane, 1, finish());
    lane.on_host_clock(1_000 + WINDOW_MS - 1);
    for ticket in &tickets {
        lane.on_assessment(ticket, usable())
            .expect("due inside the window");
    }
    let line = tasks_named(&lane, "line-01").remove(0);
    assert_eq!(line.judgment, Judgment::Met as i32);
    assert_eq!(line.execution, wire::MissionTaskExecution::Completed as i32);
    assert_eq!(collection(&line), wire::MissionEvidenceCollection::Settled);
}

#[test]
fn an_answer_after_the_window_is_late_and_changes_no_judgment() {
    let (mut lane, _) = running_lane();
    lane.on_host_clock(1_000);
    let tickets = sent_on_line(&mut lane, 1);
    switch_after_line(&mut lane, 1, finish());
    lane.on_assessment(&tickets[0], usable()).unwrap();
    lane.on_host_clock(1_000 + WINDOW_MS);
    let line = tasks_named(&lane, "line-01").remove(0);
    assert_eq!(collection(&line), wire::MissionEvidenceCollection::Closed);
    assert_eq!(line.judgment, Judgment::NotMet as i32, "one of two usable");
    assert!(matches!(
        lane.on_assessment(&tickets[1], usable()),
        Err(VerdictRefused::Late { .. })
    ));
    let line = tasks_named(&lane, "line-01").remove(0);
    let evidence = line.evidence.clone().unwrap_or_default();
    assert_eq!((evidence.late, evidence.usable), (1, 1));
    assert_eq!(
        line.judgment,
        Judgment::NotMet as i32,
        "the judgment stands"
    );
}

#[test]
fn the_window_closes_while_the_run_holds() {
    let (mut lane, _) = running_lane();
    lane.on_host_clock(1_000);
    let tickets = sent_on_line(&mut lane, 1);
    lane.on_progress(&toward(&lane, 3));
    lane.on_intent(&intent(Action::Hold(wire::MissionHold {})), &context("h"));
    lane.on_progress(&RunProgress {
        holding: true,
        ..toward(&lane, 3)
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Held);
    lane.on_host_clock(1_000 + WINDOW_MS);
    let line = tasks_named(&lane, "line-01").remove(0);
    assert_eq!(collection(&line), wire::MissionEvidenceCollection::Closed);
    assert!(matches!(
        lane.on_assessment(&tickets[0], usable()),
        Err(VerdictRefused::Late { .. })
    ));
}

#[test]
fn too_many_replaced_runs_waiting_close_the_oldest() {
    let (mut lane, _) = running_lane();
    let replaced = super::super::MAX_SETTLING_RUNS + 1;
    for _ in 0..replaced {
        sent_on_line(&mut lane, 1);
        switch_after_line(
            &mut lane,
            1,
            wire::mission_revise::Change::Objective(objective()),
        );
    }
    let lines = tasks_named(&lane, "line-01");
    let collections: Vec<_> = lines.iter().take(replaced).map(collection).collect();
    assert_eq!(collections[0], wire::MissionEvidenceCollection::Closed);
    assert!(
        collections[1..]
            .iter()
            .all(|c| *c == wire::MissionEvidenceCollection::Open),
        "{collections:?}"
    );
}
