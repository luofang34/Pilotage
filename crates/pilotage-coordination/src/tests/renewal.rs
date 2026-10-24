//! A renewal of a running team's approval: the team passes it to each member
//! that may still fly, as a renewal of that member's own plan and run, and a
//! member's renewal ends only the hold its expiry applied.

use pilotage_protocol::wire;
use wire::MissionHoldReason as Reason;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;

/// The time the team's approval, and so each member's, expires.
const EXPIRY: u64 = NOW + 3_600_000;

/// Reports `member`'s status to the team.
fn report(fleet: &mut Fleet, member: &str) {
    let status = fleet.members.get_mut(member).and_then(Lane::take_changed);
    if let Some(status) = status {
        let intents = fleet.team.on_member_status(&status, &context("report"));
        fleet.route(intents);
    }
}

/// Flies `member`'s plan to its end, every line with usable pictures.
fn complete(fleet: &mut Fleet, member: &str) {
    let lane = fleet.members.get_mut(member).expect("member");
    let lines = lane.task_ids().len().saturating_sub(1);
    for line in 0..lines {
        lane.on_progress(&toward(lane, 2 * line + 1));
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
    }
    lane.on_progress(&RunProgress {
        plan_complete: true,
        ..toward(lane, 2 * lines)
    });
    report(fleet, member);
}

/// `member`'s vehicle reports that it holds in place.
fn holds(fleet: &mut Fleet, member: &str) {
    let lane = fleet.members.get_mut(member).expect("member");
    lane.on_progress(&RunProgress {
        holding: true,
        ..toward(lane, 1)
    });
    report(fleet, member);
}

fn state(fleet: &Fleet, member: &str) -> wire::MissionLaneState {
    fleet.members.get(member).map(Lane::state).expect("member")
}

#[test]
fn a_team_renewal_resumes_only_the_members_its_expiry_held() {
    let (mut fleet, _) = running_fleet(false);
    complete(&mut fleet, "SIM-1");
    // A person holds SIM-3 at its own lane.
    let sim3 = fleet.members.get_mut("SIM-3").expect("SIM-3");
    let hold = sim3.on_intent(
        &wire::MissionIntent {
            request_id: "hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("h"),
    );
    assert_eq!(hold.effect, Some(LaneEffect::Hold));
    holds(&mut fleet, "SIM-3");
    // The approval expires on SIM-2 and SIM-3.
    for member in ["SIM-2", "SIM-3"] {
        fleet
            .members
            .get_mut(member)
            .expect("member")
            .on_clock(EXPIRY);
        holds(&mut fleet, member);
    }
    let status = fleet.team.status();
    assert_eq!(status.state, wire::MissionLaneState::Held as i32);
    assert_eq!(
        status.holds,
        vec![Reason::Operator as i32, Reason::ApprovalExpired as i32]
    );

    let mut renewal = approval(&status.proposal_digest, 2);
    renewal.run_id.clone_from(&status.run_id);
    if let Some(envelope) = renewal.envelope.as_mut() {
        envelope.expires_unix_ms = EXPIRY + 3_600_000;
    }
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "renew".to_owned(),
            action: Some(Action::Approve(renewal)),
        },
        &context("renew"),
    );
    let mut sent: Vec<&str> = reply.intents.iter().map(|i| i.member.as_str()).collect();
    sent.sort_unstable();
    assert_eq!(sent, ["SIM-2", "SIM-3"], "SIM-1 finished; it gets none");
    let decided = fleet.route(reply.intents);
    assert!(decided.first().is_some_and(accepted), "{decided:?}");
    assert_eq!(state(&fleet, "SIM-2"), wire::MissionLaneState::Running);
    assert_eq!(state(&fleet, "SIM-3"), wire::MissionLaneState::Held);
    let sim3 = fleet.members.get("SIM-3").map(Lane::status).expect("SIM-3");
    assert_eq!(sim3.holds, vec![Reason::Operator as i32]);
    assert_eq!(state(&fleet, "SIM-1"), wire::MissionLaneState::Complete);
}

#[test]
fn a_team_renewal_for_another_run_is_refused() {
    let (mut fleet, _) = running_fleet(false);
    let status = fleet.team.status();
    let mut renewal = approval(&status.proposal_digest, 2);
    renewal.run_id = "an-earlier-team-run".to_owned();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "renew".to_owned(),
            action: Some(Action::Approve(renewal)),
        },
        &context("renew"),
    );
    let receipt = reply.receipt.expect("answered at once");
    assert!(!accepted(&receipt));
    assert!(
        receipt.reason.contains("a renewal names the run"),
        "{}",
        receipt.reason
    );
    assert!(reply.intents.is_empty());
}
