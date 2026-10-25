//! What stays in force around a team revision: a withdrawn revision gives
//! back the approval it replaced, and an applied one makes its objective
//! the team's.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::{reassign, running_fleet, team_intent};
use super::*;

/// The team's survey kept to a strip around its first `lines` unflown
/// lines, of `member` when one is named, at the team's height plus
/// `climb_m`.
fn strip_objective(
    fleet: &Fleet,
    member: Option<&str>,
    lines: usize,
    climb_m: f64,
) -> wire::MissionObjective {
    use crate::allocation::{Across, sweep_of};
    let Some(wire::mission_objective::Kind::Survey(survey)) = objective().kind else {
        panic!("a survey");
    };
    let d = defaults();
    let sweep = sweep_of(&survey, d.height_m, d.across_track_fov_deg, d.sidelap);
    let across = Across::new(&survey.area, sweep.bearing_deg);
    let mut unflown: Vec<f64> = fleet
        .team
        .status()
        .tasks
        .iter()
        .filter(|t| {
            t.kind == wire::MissionTaskKind::SurveyLine as i32
                && t.state == wire::MissionTaskState::Pending as i32
                && member.is_none_or(|m| t.member == m)
        })
        .filter_map(|t| across.of_path(&t.path))
        .collect();
    unflown.sort_by(f64::total_cmp);
    assert!(unflown.len() > lines);
    let strip = across.strip(&survey.area, &unflown[..lines], sweep.footprint_m);
    wire::MissionObjective {
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: strip,
                height_m: d.height_m + climb_m,
                ..survey
            },
        )),
        ..objective()
    }
}

fn revise_objective(fleet: &Fleet, objective: wire::MissionObjective, within: bool) -> Action {
    let status = fleet.team.status();
    Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope: within,
        drafted_by: None,
        release: None,
        share_of: String::new(),
        change: Some(wire::mission_revise::Change::Objective(objective)),
    })
}

fn successor_state(fleet: &Fleet) -> Option<i32> {
    fleet.team.status().successor.map(|s| s.state)
}

/// SIM-1 stops, so it refuses its share of a revision when it arrives.
fn sim1_stops(fleet: &mut Fleet) {
    stops(fleet, "SIM-1");
}

/// `member` stops, so it refuses its share of a revision when it arrives.
fn stops(fleet: &mut Fleet, member: &str) {
    let lane = fleet.members.get_mut(member).expect("member");
    lane.on_intent(
        &wire::MissionIntent {
            request_id: "stop".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("stop"),
    );
}

/// An approval of `digest` that also permits moving lines.
fn allocating(digest: &[u8]) -> wire::MissionApproval {
    let mut wider = approval(digest, 0);
    if let Some(envelope) = wider.envelope.as_mut() {
        envelope.permit_allocate = true;
    }
    wider
}

/// Asks the team, inside its envelope, to move SIM-3's lines to SIM-2: it
/// applies only under an approval that permits moving lines.
fn assert_judged_by_the_plans_approval(fleet: &mut Fleet) {
    let action = reassign(fleet, "SIM-3", "SIM-2", 1.0, true);
    let (receipt, intents) = team_intent(fleet, action);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    assert!(intents.is_empty(), "nothing applies: {intents:?}");
    let detail = fleet.team.status().detail;
    assert!(detail.contains("permit_allocate"), "{detail}");
}

#[test]
fn a_withdrawn_revision_gives_back_the_plans_approval() {
    // The plan's approval does not permit moving lines between members.
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(allocating(&draft.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    sim1_stops(&mut fleet);
    fleet.route(intents);
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Withdrawn as i32)
    );

    // A revision inside the envelope is judged by the plan's approval again,
    // not by the withdrawn revision's.
    assert_judged_by_the_plans_approval(&mut fleet);
}

#[test]
fn a_refused_share_that_moves_no_lines_leaves_the_others_revised() {
    let (mut fleet, _) = running_fleet(false);
    let status = fleet.team.status();
    let finish = Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope: false,
        drafted_by: None,
        release: None,
        share_of: String::new(),
        change: Some(wire::mission_revise::Change::Finish(wire::MissionFinish {})),
    });
    let (receipt, _) = team_intent(&mut fleet, finish);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(allocating(&draft.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    sim1_stops(&mut fleet);
    fleet.route(intents);
    // SIM-1 keeps its work; the others apply their shares, so the revision
    // stays applied, under the approval that admitted it.
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Applied as i32)
    );
    for member in ["SIM-2", "SIM-3"] {
        let lane = fleet.members.get(member).map(Lane::status).expect("member");
        assert_eq!(
            lane.successor.as_ref().map(|s| s.state),
            Some(wire::MissionSuccessorState::Admitted as i32),
            "{member}: {}",
            lane.detail
        );
    }
}

#[test]
fn a_member_stated_down_never_answers_its_share() {
    use super::revision::member_down;
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, _unanswered) =
        team_intent(&mut fleet, Action::Approve(allocating(&draft.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    // SIM-1's link goes before its share arrives.
    fleet.team.on_member_lost("SIM-1");
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Applied as i32),
        "it may still answer"
    );

    // A person states it down: its share will not come.
    let (receipt, _) = team_intent(&mut fleet, member_down("SIM-1"));
    assert!(receipt.is_none_or(|r| accepted(&r)));
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Withdrawn as i32)
    );
    let handover = fleet.team.status().handovers.pop().expect("a handover");
    assert_eq!(handover.state, wire::MissionHandoverState::Withdrawn as i32);
    assert_judged_by_the_plans_approval(&mut fleet);
}

#[test]
fn a_late_refusal_withdraws_only_its_own_revision() {
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let first = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(allocating(&first.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));

    // Another draft comes before SIM-1 answers its share of the first.
    let action = reassign(&fleet, "SIM-3", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    sim1_stops(&mut fleet);
    fleet.route(intents);
    // The second draft still waits for its review, and the first one's
    // approval gave way to the plan's.
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Drafted as i32)
    );
    assert_judged_by_the_plans_approval(&mut fleet);
}

#[test]
fn an_applied_revision_makes_its_objective_the_teams() {
    let (mut fleet, _) = running_fleet(false);
    let strip = strip_objective(&fleet, None, 3, 0.0);
    let action = revise_objective(&fleet, strip.clone(), false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    assert_eq!(
        draft.objective.as_ref(),
        Some(&strip),
        "the reviewer reads the revised objective"
    );
    // The reviewer's approval does not permit an objective change inside
    // the envelope.
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    fleet.route(intents);
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Applied as i32)
    );

    // The same strip again changes no objective: it is the team's own now,
    // so the envelope admits it.
    let action = revise_objective(&fleet, strip, true);
    let (receipt, _) = team_intent(&mut fleet, action);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    let status = fleet.team.status();
    assert!(
        !status.detail.contains("permit_change_objective"),
        "{}",
        status.detail
    );
    assert_eq!(
        status.successor.map(|s| s.state),
        Some(wire::MissionSuccessorState::Applied as i32),
        "{}",
        status.detail
    );
}

#[test]
fn moved_lines_that_wait_fly_over_the_planned_area() {
    use super::handover::{complete, move_sim1_to_sim2, sim1_switches};
    let (mut fleet, _) = running_fleet(true);
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let sent = fleet.team.on_member_status(&status, &context("released"));
    assert!(sent.is_empty(), "SIM-2 still flies its own plan: {sent:?}");
    let moved = handover_moved(&fleet);

    // A strip that leaves the moved lines outside it applies while they wait.
    let strip = strip_objective(&fleet, Some("SIM-3"), 2, 0.0);
    let action = revise_objective(&fleet, strip, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    let draft = opened(&receipt).expect("the draft");
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    fleet.route(intents);

    complete(&mut fleet, "SIM-2");
    let sim2 = fleet.members.get("SIM-2").map(Lane::status).expect("SIM-2");
    let lines = sim2
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .count();
    assert_eq!(lines, moved, "{}", sim2.detail);
}

fn handover_moved(fleet: &Fleet) -> usize {
    let handover = fleet.team.status().handovers.pop().expect("a handover");
    handover.task_ids.len()
}

#[test]
fn a_giver_has_one_open_reassignment_at_a_time() {
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 0.5, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, _unanswered) =
        team_intent(&mut fleet, Action::Approve(allocating(&draft.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    let action = reassign(&fleet, "SIM-1", "SIM-3", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let receipt = receipt.expect("answered");
    assert!(!accepted(&receipt));
    assert!(
        receipt.reason.contains("has not released"),
        "{}",
        receipt.reason
    );
}

#[test]
fn withdrawn_revisions_in_any_order_give_back_the_plans_approval() {
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let first = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, first_shares) =
        team_intent(&mut fleet, Action::Approve(allocating(&first.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    // The second revision replaces the first one's approval.
    let action = reassign(&fleet, "SIM-2", "SIM-3", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let second = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, second_shares) =
        team_intent(&mut fleet, Action::Approve(allocating(&second.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));

    // The first one's giver refuses first, then the second one's.
    sim1_stops(&mut fleet);
    fleet.route(first_shares);
    stops(&mut fleet, "SIM-2");
    fleet.route(second_shares);
    assert_judged_by_the_plans_approval(&mut fleet);
}

#[test]
fn withdrawn_revisions_in_the_other_order_give_back_the_plans_approval() {
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let first = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, first_shares) =
        team_intent(&mut fleet, Action::Approve(allocating(&first.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    let action = reassign(&fleet, "SIM-2", "SIM-3", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let second = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, second_shares) =
        team_intent(&mut fleet, Action::Approve(allocating(&second.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));

    stops(&mut fleet, "SIM-2");
    fleet.route(second_shares);
    sim1_stops(&mut fleet);
    fleet.route(first_shares);
    assert_judged_by_the_plans_approval(&mut fleet);
}

#[test]
fn a_giver_whose_run_ends_unreleased_keeps_its_work() {
    let (mut fleet, _) = running_fleet(false);
    let action = reassign(&fleet, "SIM-1", "SIM-2", 0.5, false);
    let (receipt, _) = team_intent(&mut fleet, action);
    let draft = opened(&receipt.expect("answered")).expect("the draft");
    let (receipt, shares) = team_intent(&mut fleet, Action::Approve(allocating(&draft.digest)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    fleet.route(shares);
    // SIM-1 takes its share, then its own operator ends its run before the
    // line in progress ends, so it releases nothing.
    let sim1 = fleet.members.get_mut("SIM-1").expect("SIM-1");
    sim1.on_intent(
        &wire::MissionIntent {
            request_id: "stop".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("stop"),
    );
    let status = sim1.take_changed().expect("changed");
    fleet.team.on_member_status(&status, &context("ended"));
    let handover = fleet.team.status().handovers.pop().expect("a handover");
    assert_eq!(handover.state, wire::MissionHandoverState::Withdrawn as i32);
    assert_eq!(
        successor_state(&fleet),
        Some(wire::MissionSuccessorState::Withdrawn as i32)
    );
    assert_judged_by_the_plans_approval(&mut fleet);
}
