//! A team revision mid-run: lines move from one member to another, the
//! giving member switches at the end of its line in progress, and the
//! receiving member flies the moved lines after its own.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

/// Every member flies toward its line 1, the line in progress, and reports
/// it to the team.
fn on_line_one(fleet: &mut Fleet) {
    let mut statuses = Vec::new();
    for lane in fleet.members.values_mut() {
        lane.on_progress(&RunProgress { ..toward(lane, 1) });
        statuses.extend(lane.take_changed());
    }
    for status in statuses {
        let intents = fleet.team.on_member_status(&status, &context("line-1"));
        fleet.route(intents);
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
        .proposal_record
        .clone()
        .and_then(open_record)
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
        release: None,
        share_of: String::new(),
        change: Some(wire::mission_revise::Change::Reassign(
            wire::MissionReassign {
                from_member: from.to_owned(),
                to_member: to.to_owned(),
                fraction,
            },
        )),
    })
}

/// SIM-1 ends its line 1, with usable pictures, and switches to its revision.
fn switch_sim1(fleet: &mut Fleet) {
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    for _ in 0..3 {
        if let Some(ticket) = lane.on_frame() {
            lane.on_sent(&ticket);
            let usable = Some(FrameAssessment {
                usable: true,
                hazard: false,
            });
            lane.on_assessment(&ticket, usable).unwrap();
        }
    }
    let effect = lane.on_progress(&RunProgress { ..toward(lane, 2) });
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
    let planned_sim2: Vec<String> = fleet
        .team
        .status()
        .tasks
        .iter()
        .filter(|t| t.member == "SIM-2")
        .map(|t| t.task_id.clone())
        .collect();
    let reassign = reassign(&fleet, "SIM-1", "SIM-2", 0.5, false);
    let (receipt, intents) = team_intent(&mut fleet, reassign);
    let receipt = receipt.expect("answered at once");
    assert!(accepted(&receipt), "{}", receipt.reason);
    assert!(intents.is_empty(), "a draft changes nothing yet");
    let draft = opened(&receipt).expect("the draft for review");
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
    // The moved lines are tasks of SIM-2's own later run, which the team follows.
    assert!(team.tasks.iter().any(|t| {
        t.member == "SIM-2"
            && t.state == wire::MissionTaskState::Done as i32
            && !planned_sim2.contains(&t.task_id)
    }));
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
        release: None,
        share_of: String::new(),
        change: Some(wire::mission_revise::Change::Objective(objective)),
    });
    let (receipt, _) = team_intent(&mut fleet, revise);
    let receipt = receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    let draft = opened(&receipt).expect("the draft");
    assert_eq!(draft.tasks.len(), 3, "{}", draft.summary);
}

/// How the team reports `member`'s link.
fn link_of(fleet: &Fleet, member: &str) -> wire::MemberLink {
    fleet
        .team
        .status()
        .members
        .iter()
        .find(|m| m.lane == member)
        .and_then(|m| wire::MemberLink::try_from(m.member_link).ok())
        .unwrap_or(wire::MemberLink::Unspecified)
}

fn member_down(member: &str) -> Action {
    Action::MemberDown(wire::MissionMemberDown {
        member: member.to_owned(),
        stated_by: None,
    })
}

#[test]
fn a_lost_member_keeps_its_work_until_it_reports_or_a_person_states_it_down() {
    let (mut fleet, _) = running_fleet(false);
    assert_eq!(link_of(&fleet, "SIM-3"), wire::MemberLink::Reporting);
    // SIM-3 is only partitioned: it keeps flying its line 1.
    fleet.team.on_member_lost("SIM-3");
    assert_eq!(link_of(&fleet, "SIM-3"), wire::MemberLink::Lost);
    let give = reassign(&fleet, "SIM-3", "SIM-1", 1.0, false);
    let (receipt, intents) = team_intent(&mut fleet, give);
    let receipt = receipt.expect("answered");
    assert!(!accepted(&receipt));
    assert!(
        receipt.reason.contains("stays reserved"),
        "{}",
        receipt.reason
    );
    assert!(intents.is_empty());
    // A member that reports cannot be stated down.
    let (receipt, _) = team_intent(&mut fleet, member_down("SIM-1"));
    assert!(!receipt.is_some_and(|r| accepted(&r)));
    // SIM-3 reports again: a reassignment moves only the lines it has not begun.
    let sim3 = fleet.members.get("SIM-3").expect("SIM-3").status();
    fleet.team.on_member_status(&sim3, &context("back"));
    let give = reassign(&fleet, "SIM-3", "SIM-1", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, give);
    let draft = opened(&receipt.expect("answered")).expect("a draft");
    let line_in_progress = sim3
        .tasks
        .iter()
        .find(|t| t.state == wire::MissionTaskState::Active as i32)
        .expect("SIM-3 flies a line");
    let moved_paths: Vec<_> = draft
        .tasks
        .iter()
        .filter(|t| t.member == "SIM-1")
        .map(|t| t.path.clone())
        .collect();
    assert!(
        !moved_paths.contains(&line_in_progress.path),
        "the line in progress stays with SIM-3"
    );
    // Its link goes again before the approval: the draft waits.
    fleet.team.on_member_lost("SIM-3");
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    let receipt = receipt.expect("answered");
    assert!(!accepted(&receipt), "{}", receipt.reason);
    assert!(intents.is_empty());
}

#[test]
fn a_person_states_a_lost_member_down_and_the_team_completes_without_it() {
    let (mut fleet, planned) = running_fleet(false);
    // SIM-3's host is gone mid-line: the team keeps its work reserved.
    fleet.team.on_member_lost("SIM-3");
    fleet.members.remove("SIM-3");
    let (receipt, _) = team_intent(&mut fleet, member_down("SIM-3"));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    assert_eq!(link_of(&fleet, "SIM-3"), wire::MemberLink::Down);
    let give = reassign(&fleet, "SIM-3", "SIM-1", 1.0, false);
    let (receipt, intents) = team_intent(&mut fleet, give);
    let draft = opened(&receipt.expect("answered")).expect("a draft");
    assert!(intents.is_empty());
    let (receipt, intents) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    assert!(
        intents.iter().all(|i| i.member != "SIM-3"),
        "nothing goes to a member that cannot hear it"
    );
    assert_eq!(link_of(&fleet, "SIM-3"), wire::MemberLink::Released);
    assert!(
        fleet.team.status().detail.contains("SIM-3 is down"),
        "{}",
        fleet.team.status().detail
    );
    let sim3 = fleet
        .team
        .status()
        .members
        .into_iter()
        .find(|m| m.lane == "SIM-3");
    assert!(sim3.is_some_and(|m| m.detail.contains("down")));
    // The host keeps failing to reconnect; the statement stands.
    fleet.team.on_member_lost("SIM-3");
    let sim3 = fleet
        .team
        .status()
        .members
        .into_iter()
        .find(|m| m.lane == "SIM-3");
    assert!(sim3.is_some_and(|m| m.detail.contains("down")));
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

#[test]
fn a_fetch_returns_the_team_plan_or_its_drafted_revision() {
    let (mut fleet, _) = running_fleet(false);
    let plan = fleet.team.status().proposal_digest;
    let fetch = |digest: &[u8]| {
        Action::Fetch(wire::MissionFetch {
            digest: digest.to_vec(),
        })
    };
    let (receipt, _) = team_intent(&mut fleet, fetch(&plan));
    let fetched = opened(&receipt.expect("answered")).expect("the plan");
    assert_eq!(fetched.digest, plan);
    let give = reassign(&fleet, "SIM-2", "SIM-1", 0.5, false);
    let (receipt, _) = team_intent(&mut fleet, give);
    let draft = opened(&receipt.expect("answered")).expect("a draft");
    let (receipt, _) = team_intent(&mut fleet, fetch(&draft.digest));
    assert_eq!(
        opened(&receipt.expect("answered")).map(|p| p.digest),
        Some(draft.digest)
    );
}

#[test]
fn a_team_proposal_carries_each_member_record_unchanged() {
    let mut fleet = Fleet::new();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-1".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("team-proposal"),
    );
    let decided = fleet.route(reply.intents);
    let record = decided[0].proposal_record.clone().expect("a team record");
    let team = open_record(record).expect("the team record verifies");
    assert!(team.requires.iter().any(|f| f.contains("team-of-lanes")));
    for member in &team.members {
        let embedded = member.record.clone().expect("the member's record");
        let lane = fleet.members.get_mut(&member.member).expect("member");
        let fetch = wire::MissionIntent {
            request_id: "fetch".to_owned(),
            action: Some(Action::Fetch(wire::MissionFetch { digest: Vec::new() })),
        };
        let own = lane
            .on_intent(&fetch, &context("f"))
            .receipt
            .proposal_record;
        assert_eq!(Some(embedded), own, "{}'s bytes, unchanged", member.member);
    }
}
