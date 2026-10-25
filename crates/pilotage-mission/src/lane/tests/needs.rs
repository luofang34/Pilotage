//! The approvals a lane needs, and the team that owes one when the
//! approval in force came down from it.

use pilotage_protocol::wire;
use wire::MissionApprovalTarget as Target;
use wire::mission_intent::Action;

use super::*;

fn needs(lane: &Lane) -> Vec<(Target, String)> {
    lane.status()
        .approvals_needed
        .iter()
        .map(|n| (Target::try_from(n.target).unwrap(), n.owed_by_lane.clone()))
        .collect()
}

/// A running lane whose approval came from `granted_by`, passed down by the
/// team `delegated_by` (empty for a person's approval).
fn running_under(granted_by: &str, delegated_by: &str) -> Lane {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    assert_eq!(needs(&lane), [(Target::Plan, String::new())]);
    let mut approved = approval(&digest, 120.0);
    approved.delegated_by = delegated_by.to_owned();
    approved.approver = Some(wire::RecordOrigin {
        subject: granted_by.to_owned(),
        ..wire::RecordOrigin::default()
    });
    lane.on_intent(&intent(Action::Approve(approved)), &context("a"));
    lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: digest,
        })),
        &context("run-1"),
    );
    lane.on_progress(&toward(&lane, 1));
    assert!(needs(&lane).is_empty(), "a running lane needs nothing");
    lane
}

/// Drafts an end after the line in progress, as the share of the team
/// `share_of` (empty for no team's).
fn draft_finish(lane: &mut Lane, share_of: &str) {
    let status = lane.status();
    lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            within_envelope: false,
            drafted_by: None,
            release: None,
            share_of: share_of.to_owned(),
            change: Some(wire::mission_revise::Change::Finish(wire::MissionFinish {})),
        })),
        &context("rev"),
    );
}

#[test]
fn a_persons_lane_needs_its_approvals_from_any_approver() {
    let mut lane = running_under("operator", "");
    draft_finish(&mut lane, "");
    assert_eq!(needs(&lane), [(Target::Successor, String::new())]);
    let mut lane = running_under("operator", "");
    lane.on_clock(NOW + 3_600_000);
    assert_eq!(needs(&lane), [(Target::Renewal, String::new())]);
}

#[test]
fn a_team_owes_its_own_share_and_the_renewal_of_what_it_granted() {
    let mut lane = running_under("coordinator", "TEAM");
    draft_finish(&mut lane, "TEAM");
    assert_eq!(needs(&lane), [(Target::Successor, "TEAM".to_owned())]);
    // A revision that is not that team's share is for a person.
    let mut lane = running_under("coordinator", "TEAM");
    draft_finish(&mut lane, "");
    assert_eq!(needs(&lane), [(Target::Successor, String::new())]);
    // The share of another team than the one that granted the approval too.
    let mut lane = running_under("coordinator", "TEAM");
    draft_finish(&mut lane, "OTHER");
    assert_eq!(needs(&lane), [(Target::Successor, String::new())]);
    let mut lane = running_under("coordinator", "TEAM");
    lane.on_clock(NOW + 3_600_000);
    assert_eq!(needs(&lane), [(Target::Renewal, "TEAM".to_owned())]);
}

#[test]
fn a_plan_a_team_made_for_its_member_is_the_teams_to_approve() {
    let mut lane = Lane::new(config());
    let objective = wire::MissionObjective {
        delegated_by: "TEAM".to_owned(),
        ..objective()
    };
    lane.on_intent(&intent(Action::Propose(objective)), &context("p"));
    assert_eq!(needs(&lane), [(Target::Plan, "TEAM".to_owned())]);
}

#[test]
fn a_lane_that_ended_needs_no_approval() {
    let mut lane = running_under("operator", "");
    draft_finish(&mut lane, "");
    lane.on_clock(NOW + 3_600_000);
    assert_eq!(
        needs(&lane).len(),
        2,
        "a draft and a renewal while it holds"
    );
    lane.on_progress(&RunProgress {
        ended: true,
        ended_because: Some("the vehicle landed"),
        ..toward(&lane, 1)
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Failed);
    assert!(needs(&lane).is_empty(), "{:?}", needs(&lane));
}
