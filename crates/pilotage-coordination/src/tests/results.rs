//! A team's runs ending is not its results settling: the team waits for
//! the evidence of its lines, a judgment that comes inside the window still
//! flies a line again under an approval valid then, and a judgment the
//! window closed stays closed.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;

const LINE: (&str, &str) = ("SIM-2", "line-01");

/// Gives each held picture its judgment and the team the member's status
/// at `now_unix_ms`. Returns the member intents the team sends.
fn judge(
    fleet: &mut Fleet,
    held: &[(String, FrameTicket)],
    usable: bool,
    now_unix_ms: u64,
) -> Vec<MemberIntent> {
    let mut intents = Vec::new();
    for (member, ticket) in held {
        let lane = fleet.members.get_mut(member).expect("member");
        // A judgment after the window is refused and kept apart.
        lane.on_assessment(
            ticket,
            Some(FrameAssessment {
                usable,
                hazard: false,
            }),
        )
        .ok();
    }
    for member in held
        .iter()
        .map(|(m, _)| m.clone())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let status = fleet.members.get_mut(&member).and_then(Lane::take_changed);
        if let Some(status) = status {
            let at = IntentContext {
                now_unix_ms,
                fresh_id: "judged".to_owned(),
            };
            intents.extend(fleet.team.on_member_status(&status, &at));
        }
    }
    intents
}

fn line(fleet: &Fleet) -> wire::MissionTaskStatus {
    fleet
        .team
        .status()
        .tasks
        .into_iter()
        .find(|t| {
            t.task_id == format!("{}/{}", LINE.0, LINE.1)
                || (t.member == LINE.0 && t.name.ends_with(LINE.1))
        })
        .expect("the line")
}

/// Flies every member's plan to its end; SIM-2's first line waits for the
/// judgment of its pictures.
fn flown_waiting() -> (Fleet, Vec<(String, FrameTicket)>) {
    let (mut fleet, _) = running_fleet(false);
    let (intents, held) = fleet.fly_holding(&[], Some(LINE));
    assert!(intents.is_empty(), "nothing to fly again yet");
    assert!(!held.is_empty(), "the line's pictures wait for a judgment");
    (fleet, held)
}

#[test]
fn a_team_whose_runs_ended_waits_for_its_results_and_says_so() {
    let (fleet, _) = flown_waiting();
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Running as i32,
        "{}",
        status.detail
    );
    assert_eq!(
        status.execution.map(|e| e.state),
        Some(wire::MissionExecutionState::AwaitingResults as i32)
    );
    assert!(
        status.detail.contains("waiting for the results"),
        "{}",
        status.detail
    );
}

#[test]
fn bad_evidence_after_the_last_run_ended_still_flies_the_line_again() {
    let (mut fleet, held) = flown_waiting();
    let refly = judge(&mut fleet, &held, false, NOW);
    assert_eq!(refly.len(), 1, "one re-flight");
    assert_ne!(refly[0].member, LINE.0, "not the camera that failed");
    assert_eq!(
        line(&fleet).judgment,
        wire::MissionTaskJudgment::NotMet as i32
    );
}

#[test]
fn the_same_results_give_the_same_judgment_before_or_after_the_runs_end() {
    let (mut early, _) = running_fleet(false);
    let early_refly = early.fly(&[LINE]);
    let (mut late, held) = flown_waiting();
    let late_refly = judge(&mut late, &held, false, NOW);
    assert_eq!(line(&early).judgment, line(&late).judgment);
    assert_eq!(early_refly.len(), late_refly.len());
    assert_eq!(
        early_refly.first().map(|i| &i.member),
        late_refly.first().map(|i| &i.member)
    );
}

#[test]
fn a_judgment_after_the_window_does_not_change_a_closed_one() {
    let (mut fleet, held) = flown_waiting();
    // The window closes with no answer: the judgment uses what arrived.
    let lane = fleet.members.get_mut(LINE.0).expect("member");
    lane.on_host_clock(10 * 60_000);
    let status = lane.take_changed().expect("the window closed");
    fleet.team.on_member_status(&status, &context("closed"));
    let closed = line(&fleet);
    assert_eq!(
        closed.evidence.as_ref().map(|e| e.collection),
        Some(wire::MissionEvidenceCollection::Closed as i32)
    );
    judge(&mut fleet, &held, true, NOW);
    let after = line(&fleet);
    assert_eq!(
        after.judgment, closed.judgment,
        "the late answers change nothing"
    );
}

#[test]
fn bad_evidence_after_the_approval_expired_is_recorded_and_flies_nothing() {
    let (mut fleet, held) = flown_waiting();
    let expired = NOW + 3_600_000;
    let intents = judge(&mut fleet, &held, false, expired);
    assert!(intents.is_empty(), "no approval valid now flies it again");
    assert_eq!(
        line(&fleet).judgment,
        wire::MissionTaskJudgment::NotMet as i32
    );
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Complete as i32,
        "the team ends: nothing waits any more"
    );
}

#[test]
fn results_after_an_abort_are_recorded_and_start_nothing() {
    let (mut fleet, held) = flown_waiting();
    let abort = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "abort".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("abort"),
    );
    assert!(abort.receipt.is_some_and(|r| accepted(&r)));
    let intents = judge(&mut fleet, &held, false, NOW);
    assert!(
        intents
            .iter()
            .all(|i| !matches!(i.intent.action, Some(Action::Propose(_) | Action::Start(_))))
    );
    assert_eq!(
        line(&fleet).judgment,
        wire::MissionTaskJudgment::NotMet as i32
    );
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Aborted as i32
    );
}

#[test]
fn a_team_whose_tasks_take_no_judgment_ends_with_its_runs() {
    let mut fleet = Fleet::new();
    for (name, lane) in &mut fleet.members {
        *lane = vehicle_lane_judged(name, false);
    }
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
    fleet.approve_and_start(approval(&proposal.digest, 2));
    let intents = fleet.fly(&[]);
    assert!(intents.is_empty());
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
    assert!(status.execution.is_none());
}

/// Routes and flies until the fleet settles.
fn fly_out(fleet: &mut Fleet, intents: Vec<MemberIntent>) {
    let mut intents = intents;
    for _ in 0..4 {
        fleet.route(intents);
        intents = fleet.fly(&[]);
    }
}

#[test]
fn a_member_that_failed_with_lines_it_never_flew_ends_the_team_failed() {
    let (mut fleet, _) = running_fleet(false);
    let lane = fleet.members.get_mut("SIM-3").expect("SIM-3");
    lane.on_progress(&RunProgress {
        ended: true,
        ended_because: Some("the vehicle landed"),
        ..toward(lane, 1)
    });
    let status = lane.take_changed().expect("SIM-3 reports");
    let intents = fleet.team.on_member_status(&status, &context("failed"));
    let more = fleet.fly(&[]);
    fly_out(&mut fleet, intents.into_iter().chain(more).collect());
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Failed as i32,
        "{}",
        status.detail
    );
    assert!(status.execution.is_none(), "nothing waits for results");
}

#[test]
fn a_re_flight_that_ends_before_its_judgments_settles_the_line_when_they_come() {
    let (mut fleet, held) = flown_waiting();
    let refly = judge(&mut fleet, &held, false, NOW);
    let flyer = refly
        .first()
        .map(|i| i.member.clone())
        .expect("a re-flight");
    fleet.route(refly);
    let name = fleet
        .members
        .get(&flyer)
        .map(Lane::status)
        .and_then(|s| {
            s.tasks
                .into_iter()
                .find(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        })
        .map(|t| t.name)
        .expect("the line flown again");
    let (intents, again) = fleet.fly_holding(&[], Some((flyer.as_str(), name.as_str())));
    assert!(intents.is_empty() && !again.is_empty());
    assert_eq!(
        fleet.team.status().execution.map(|e| e.state),
        Some(wire::MissionExecutionState::AwaitingResults as i32),
        "the re-flight ended; its verdict is open"
    );
    judge(&mut fleet, &again, true, NOW);
    assert_eq!(line(&fleet).judgment, wire::MissionTaskJudgment::Met as i32);
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
}

#[test]
fn stating_the_only_member_with_open_evidence_down_ends_a_waiting_team() {
    let (mut fleet, _) = flown_waiting();
    fleet.team.on_member_lost(LINE.0);
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "down".to_owned(),
            action: Some(Action::MemberDown(wire::MissionMemberDown {
                member: LINE.0.to_owned(),
                stated_by: None,
            })),
        },
        &context("down"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
}
