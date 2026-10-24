//! Revisions of a plan in progress: drafted for review, admitted by an
//! approval or inside the envelope, and switched to at a line's end.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

fn revise(
    status: &wire::MissionStatus,
    change: wire::mission_revise::Change,
    within_envelope: bool,
) -> wire::MissionIntent {
    intent(Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope,
        drafted_by: None,
        change: Some(change),
    }))
}

fn finish() -> wire::mission_revise::Change {
    wire::mission_revise::Change::Finish(wire::MissionFinish {})
}

fn progress(lane: &Lane, target: usize) -> RunProgress {
    toward(lane, target)
}

fn task_state(status: &wire::MissionStatus, id: &str) -> Option<wire::MissionTaskState> {
    status
        .tasks
        .iter()
        .find(|t| t.name == id)
        .and_then(|t| wire::MissionTaskState::try_from(t.state).ok())
}

#[test]
fn an_approved_revision_waits_for_the_line_in_progress_to_end() {
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    lane.on_progress(&progress(&lane, 1));
    let status = lane.status();

    let mut stale = status.clone();
    stale.proposal_digest = b"old".to_vec();
    let refused = lane.on_intent(&revise(&stale, finish(), false), &context("rev-0"));
    assert!(!accepted(&refused));
    assert!(
        refused.receipt.reason.contains("the plan is now"),
        "{}",
        refused.receipt.reason
    );

    let drafted = lane.on_intent(&revise(&status, finish(), false), &context("rev-1"));
    assert!(accepted(&drafted), "{}", drafted.receipt.reason);
    let draft = opened(&drafted.receipt).expect("the draft for review");
    assert_eq!(draft.predecessor_digest, status.proposal_digest);
    assert!(draft.summary.contains("return to the launch point"));
    let successor = lane.status().successor.expect("drafted");
    assert_eq!(successor.state, wire::MissionSuccessorState::Drafted as i32);

    let admitted = lane.on_intent(
        &intent(Action::Approve(approval(&draft.digest, 120.0))),
        &context("approve-rev"),
    );
    assert!(accepted(&admitted), "{}", admitted.receipt.reason);
    assert!(admitted.effect.is_none(), "never mid-line");
    let successor = lane.status().successor.expect("admitted");
    assert_eq!(
        successor.state,
        wire::MissionSuccessorState::Admitted as i32
    );
    assert_eq!(successor.applies_after, "line-01");
    assert!(
        lane.on_progress(&progress(&lane, 1)).is_none(),
        "line 1 still flies"
    );

    // Line 1 ends: the vehicle turns for line 2, and the revision applies.
    let Some(LaneEffect::Start(order)) = lane.on_progress(&progress(&lane, 2)) else {
        panic!("the switch at the line's end");
    };
    assert_eq!(order.plan, RunPlan::Home);
    let status = lane.status();
    assert_eq!(status.state, wire::MissionLaneState::Running as i32);
    assert_eq!(status.proposal_digest, draft.digest);
    assert_eq!(
        task_state(&status, "line-01"),
        Some(wire::MissionTaskState::Incomplete)
    );
    assert_eq!(
        task_state(&status, "line-02"),
        Some(wire::MissionTaskState::Skipped)
    );
    // The way home is a task of the new run, which names the run it continues.
    assert!(
        status
            .tasks
            .iter()
            .any(|t| { t.name == "home" && t.state == wire::MissionTaskState::Pending as i32 })
    );
    assert!(!status.predecessor_run_id.is_empty());
    assert_ne!(status.predecessor_run_id, status.run_id);
    assert_eq!(
        status.successor.map(|s| s.state),
        Some(wire::MissionSuccessorState::Applied as i32)
    );
    lane.on_progress(&RunProgress {
        plan_complete: true,
        ended: false,
        ended_because: None,
        holding: false,
        execution: None,
        ..toward(&lane, 0)
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Complete);
}

#[test]
fn inside_the_envelope_a_revision_applies_without_a_new_approval() {
    let (mut lane, _) = running_lane();
    lane.on_progress(&progress(&lane, 1));
    // The running lane's approval does not permit revisions: a draft stays.
    let status = lane.status();
    let reply = lane.on_intent(&revise(&status, finish(), true), &context("rev-1"));
    assert!(accepted(&reply));
    assert!(reply.effect.is_none());
    let status = lane.status();
    assert_eq!(
        status.successor.as_ref().map(|s| s.state),
        Some(wire::MissionSuccessorState::Drafted as i32)
    );
    assert!(
        status.detail.contains("does not permit revisions"),
        "{}",
        status.detail
    );

    // Under an approval that permits revisions, a held run switches at once.
    let mut lane = Lane::new(config());
    let proposal = lane
        .on_intent(&intent(Action::Propose(objective())), &context("p"))
        .receipt
        .proposal_record
        .and_then(open_record)
        .expect("a proposal");
    let mut permissive = approval(&proposal.digest, 120.0);
    if let Some(envelope) = permissive.envelope.as_mut() {
        envelope.permit_revise = true;
    }
    assert!(accepted(
        &lane.on_intent(&intent(Action::Approve(permissive)), &context("a"))
    ));
    lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: proposal.digest.clone(),
        })),
        &context("run-1"),
    );
    lane.on_progress(&progress(&lane, 1));
    lane.on_intent(&intent(Action::Hold(wire::MissionHold {})), &context("h"));
    let status = lane.status();
    let reply = lane.on_intent(&revise(&status, finish(), true), &context("rev-2"));
    assert!(
        matches!(reply.effect, Some(LaneEffect::Start(_))),
        "held: at once"
    );
    assert_eq!(lane.state(), wire::MissionLaneState::Running);
}

#[test]
fn before_a_start_a_revised_objective_replaces_the_draft() {
    let mut lane = Lane::new(config());
    lane.on_intent(&intent(Action::Propose(objective())), &context("p1"));
    let status = lane.status();
    let mut revised = objective();
    revised.title = "smaller".to_owned();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = revised.kind.as_mut() {
        survey.height_m = 50.0;
    }
    let reply = lane.on_intent(
        &revise(
            &status,
            wire::mission_revise::Change::Objective(revised),
            false,
        ),
        &context("p2"),
    );
    assert!(accepted(&reply), "{}", reply.receipt.reason);
    let replaced = opened(&reply.receipt).expect("the new draft");
    assert_ne!(replaced.digest, status.proposal_digest);
    assert_eq!(lane.state(), wire::MissionLaneState::Proposed);
    let finish_before_start =
        lane.on_intent(&revise(&lane.status(), finish(), false), &context("p3"));
    assert!(!accepted(&finish_before_start));
}

#[test]
fn between_lines_a_revision_applies_at_once() {
    let (mut lane, _) = running_lane();
    // Toward the start of line 1: no line is in progress.
    lane.on_progress(&progress(&lane, 0));
    let status = lane.status();
    let draft = lane
        .on_intent(&revise(&status, finish(), false), &context("rev-1"))
        .receipt
        .proposal_record
        .and_then(open_record)
        .expect("the draft");
    let admitted = lane.on_intent(
        &intent(Action::Approve(approval(&draft.digest, 120.0))),
        &context("approve-rev"),
    );
    assert!(
        matches!(admitted.effect, Some(LaneEffect::Start(_))),
        "a transit is cut short, so line 1 is not flown on the way"
    );
    let status = lane.status();
    assert_eq!(
        task_state(&status, "line-01"),
        Some(wire::MissionTaskState::Skipped)
    );
}

#[test]
fn an_approval_that_names_no_record_format_is_refused() {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    let mut unformatted = approval(&digest, 120.0);
    unformatted.proposal_format = wire::MissionProposalFormat::Unspecified as i32;
    let refused = lane.on_intent(&intent(Action::Approve(unformatted)), &context("a"));
    assert!(!accepted(&refused));
    assert!(
        refused.receipt.reason.contains("proposal record format"),
        "{}",
        refused.receipt.reason
    );
    assert_eq!(lane.state(), wire::MissionLaneState::Proposed);
}
