//! A team revision mid-run: lines move from one member to another, the
//! giving member switches at the end of its line in progress, and the
//! receiving member flies the moved lines after its own.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

/// Every member flies toward its line 1: the line in progress.
fn on_line_one(fleet: &mut Fleet) {
    for lane in fleet.members.values_mut() {
        lane.on_progress(&RunProgress {
            target_waypoint: Some(1),
            ..RunProgress::default()
        });
    }
}

fn team_intent(
    fleet: &mut Fleet,
    action: Action,
) -> (Option<wire::MissionReceipt>, Vec<MemberIntent>) {
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-revise".to_owned(),
            action: Some(action),
        },
        &context("revision"),
    );
    (reply.receipt, reply.intents)
}

fn lines_of(status: &wire::MissionStatus, state: wire::MissionTaskState) -> usize {
    status
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32 && t.state == state as i32)
        .count()
}

/// A team proposed, approved under `permit_allocate` or not, started,
/// and every member on its line 1; the number of lines planned.
pub(super) fn running_fleet(permit_allocate: bool) -> (Fleet, usize) {
    let mut fleet = Fleet::new();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-1".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("team-proposal"),
    );
    let proposal = fleet.route(reply.intents)[0]
        .proposal
        .clone()
        .expect("proposal");
    let planned = proposal
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .count();
    let mut team_approval = approval(&proposal.digest, 2);
    if let Some(envelope) = team_approval.envelope.as_mut() {
        envelope.permit_allocate = permit_allocate;
    }
    fleet.approve_and_start(team_approval);
    on_line_one(&mut fleet);
    (fleet, planned)
}

fn reassign(fleet: &Fleet, from: &str, to: &str, fraction: f64, within: bool) -> Action {
    let status = fleet.team.status();
    Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope: within,
        drafted_by: None,
        change: Some(wire::mission_revise::Change::Reassign(
            wire::MissionReassign {
                from_member: from.to_owned(),
                to_member: to.to_owned(),
                fraction,
            },
        )),
    })
}

/// SIM-1 ends its line 1 and switches to its revision.
fn switch_sim1(fleet: &mut Fleet) {
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    let effect = lane.on_progress(&RunProgress {
        target_waypoint: Some(2),
        ..RunProgress::default()
    });
    assert!(
        matches!(effect, Some(LaneEffect::Start(_))),
        "the switch at the line end"
    );
    assert!(lines_of(&lane.status(), wire::MissionTaskState::Skipped) > 0);
    if let Some(status) = lane.take_changed() {
        fleet.team.on_member_status(&status, &context("switch"));
    }
}

#[test]
fn half_of_a_members_unflown_lines_move_and_are_flown_once() {
    let (mut fleet, planned) = running_fleet(false);
    let reassign = reassign(&fleet, "SIM-1", "SIM-2", 0.5, false);
    let (receipt, intents) = team_intent(&mut fleet, reassign);
    let receipt = receipt.expect("answered at once");
    assert!(accepted(&receipt), "{}", receipt.reason);
    assert!(intents.is_empty(), "a draft changes nothing yet");
    let draft = receipt.proposal.expect("the draft for review");
    assert!(draft.summary.contains("to SIM-2"), "{}", draft.summary);
    let successor = fleet.team.status().successor.expect("drafted");
    assert_eq!(successor.state, wire::MissionSuccessorState::Drafted as i32);

    // The operator approves the draft; SIM-1 drafts its share, and the
    // team admits it under its delegated approval.
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    fleet.route(intents);
    let sim1 = fleet.members.get("SIM-1").map(Lane::status).expect("SIM-1");
    assert_eq!(
        sim1.successor.as_ref().map(|s| s.state),
        Some(wire::MissionSuccessorState::Admitted as i32),
        "{}",
        sim1.detail
    );

    // SIM-1 ends line 1: it switches to the lines it keeps.
    switch_sim1(&mut fleet);

    // Everyone flies to the end; SIM-2 then flies the moved lines.
    let follow = fleet.fly(&[]);
    assert_eq!(follow.len(), 1, "SIM-2 proposes the moved lines");
    assert_eq!(follow[0].member, "SIM-2");
    fleet.route(follow);
    assert_eq!(
        fleet.members.get("SIM-2").map(Lane::state),
        Some(wire::MissionLaneState::Running)
    );
    let last = fleet.fly(&[]);
    fleet.route(last);

    let team = fleet.team.status();
    assert_eq!(
        team.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        team.detail
    );
    let done = lines_of(&team, wire::MissionTaskState::Done);
    let skipped = lines_of(&team, wire::MissionTaskState::Skipped);
    assert!(skipped > 0);
    assert_eq!(
        done, planned,
        "every planned line flown once: {}",
        team.detail
    );
    assert!(team.tasks.iter().any(|t| t.task_id.contains("SIM-2/moved")));
}

#[test]
fn inside_the_envelope_a_reassignment_applies_at_once() {
    let (mut fleet, _) = running_fleet(true);
    let finish = reassign(&fleet, "SIM-3", "SIM-1", 1.0, true);
    let (receipt, intents) = team_intent(&mut fleet, finish);
    assert!(receipt.is_some_and(|r| accepted(&r)));
    assert_eq!(intents.len(), 1, "SIM-3 gets its share at once");
    fleet.route(intents);
    let sim3 = fleet.members.get("SIM-3").map(Lane::status).expect("SIM-3");
    let successor = sim3.successor.expect("SIM-3's revision");
    assert!(
        successor.summary.contains("return to the launch point"),
        "{}",
        successor.summary
    );
    assert_eq!(
        successor.state,
        wire::MissionSuccessorState::Admitted as i32
    );
}

#[test]
fn a_new_proposal_after_a_run_names_no_run_until_it_starts() {
    let (mut fleet, _) = running_fleet(false);
    let ran = fleet.team.status().run_id;
    assert!(!ran.is_empty());
    let last = fleet.fly(&[]);
    fleet.route(last);
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Complete as i32
    );
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-2".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("second-proposal"),
    );
    fleet.route(reply.intents);
    let status = fleet.team.status();
    assert_eq!(status.state, wire::MissionLaneState::Proposed as i32);
    assert!(
        status.run_id.is_empty(),
        "the new plan's tasks must not pass for run {ran}"
    );
}

#[test]
fn a_parents_strip_keeps_exactly_the_lines_inside_it() {
    use crate::allocation::{Across, sweep_of};
    let (mut fleet, _) = running_fleet(false);
    let status = fleet.team.status();
    let Some(wire::mission_objective::Kind::Survey(survey)) = objective().kind else {
        panic!("a survey");
    };
    let d = defaults();
    let sweep = sweep_of(&survey, d.height_m, d.across_track_fov_deg, d.sidelap);
    let across = Across::new(&survey.area, sweep.bearing_deg);
    let mut unflown: Vec<f64> = status
        .tasks
        .iter()
        .filter(|t| {
            t.kind == wire::MissionTaskKind::SurveyLine as i32
                && t.state == wire::MissionTaskState::Pending as i32
        })
        .filter_map(|t| across.of_path(&t.path))
        .collect();
    unflown.sort_by(f64::total_cmp);
    assert!(unflown.len() >= 4);
    // The parent keeps the first three unflown lines across the area.
    let kept = &unflown[..3];
    let strip = across.strip(&survey.area, kept, sweep.footprint_m);
    let objective = wire::MissionObjective {
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: strip,
                height_m: d.height_m,
                ..survey.clone()
            },
        )),
        ..objective()
    };
    let revise = Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope: false,
        drafted_by: None,
        change: Some(wire::mission_revise::Change::Objective(objective)),
    });
    let (receipt, _) = team_intent(&mut fleet, revise);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    let draft = receipt.proposal.expect("the draft");
    assert_eq!(draft.tasks.len(), 3, "{}", draft.summary);
}

#[test]
fn a_person_releases_a_lost_member_and_the_team_completes_without_it() {
    let (mut fleet, planned) = running_fleet(false);
    // SIM-3's host is gone mid-line: the team keeps its work reserved.
    fleet.team.on_member_lost("SIM-3");
    fleet.members.remove("SIM-3");
    let give = reassign(&fleet, "SIM-3", "SIM-1", 1.0, false);
    let (receipt, intents) = team_intent(&mut fleet, give);
    let draft = receipt.expect("answered").proposal.expect("a draft");
    assert!(intents.is_empty());
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    assert!(
        intents.iter().all(|i| i.member != "SIM-3"),
        "nothing goes to a member that cannot hear it"
    );
    assert!(
        fleet.team.status().detail.contains("SIM-3 released"),
        "{}",
        fleet.team.status().detail
    );
    fleet.route(intents);
    let follow = fleet.fly(&[]);
    fleet.route(follow);
    let last = fleet.fly(&[]);
    fleet.route(last);

    let team = fleet.team.status();
    assert_eq!(
        team.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        team.detail
    );
    let sim3: Vec<_> = team
        .tasks
        .iter()
        .filter(|t| t.member == "SIM-3" && t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .collect();
    assert!(
        sim3.iter()
            .all(|t| t.state == wire::MissionTaskState::Skipped as i32),
        "the line in flight moved too"
    );
    assert_eq!(
        lines_of(&team, wire::MissionTaskState::Done),
        planned,
        "every line flown once"
    );
}
