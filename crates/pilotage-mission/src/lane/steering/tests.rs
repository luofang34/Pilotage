use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::{HOST_ISSUER, Requester, Steering};

fn human() -> Requester {
    Requester {
        subject: "sokoly-app".to_owned(),
        may_approve: true,
    }
}

fn agent() -> Requester {
    Requester {
        subject: "mission-agent".to_owned(),
        may_approve: false,
    }
}

fn intent(action: Action) -> wire::MissionIntent {
    wire::MissionIntent {
        request_id: "r-1".to_owned(),
        action: Some(action),
    }
}

fn approve(claimed: &str) -> wire::MissionIntent {
    intent(Action::Approve(wire::MissionApproval {
        approver: Some(wire::RecordOrigin {
            issuer: "local".to_owned(),
            subject: claimed.to_owned(),
            organization: String::new(),
        }),
        ..wire::MissionApproval::default()
    }))
}

#[test]
fn only_an_approver_approves_and_the_record_names_the_session() {
    let steering = Steering::default();
    let mut forged = approve("sokoly-app");
    let refused = steering.admit(&mut forged, &agent()).unwrap_err();
    assert!(
        refused.contains("mission-agent may not approve"),
        "{refused}"
    );

    let mut claimed = approve("somebody-else");
    steering.admit(&mut claimed, &human()).unwrap();
    let Some(Action::Approve(approval)) = claimed.action else {
        panic!("the approval changed kind");
    };
    let approver = approval.approver.unwrap();
    assert_eq!(approver.issuer, HOST_ISSUER);
    assert_eq!(approver.subject, "sokoly-app");
}

#[test]
fn every_client_may_stop_and_start_an_approved_proposal() {
    let steering = Steering::default();
    for action in [
        Action::Propose(wire::MissionObjective::default()),
        Action::Start(wire::MissionStart::default()),
        Action::Hold(wire::MissionHold {}),
        Action::Abort(wire::MissionAbort {}),
        Action::Resume(wire::MissionResume {}),
    ] {
        steering.admit(&mut intent(action), &agent()).unwrap();
    }
}

#[test]
fn an_approvers_hold_waits_for_an_approver() {
    let mut steering = Steering::default();
    let hold = intent(Action::Hold(wire::MissionHold {}));
    steering.accepted(&hold, &human());
    let mut resume = intent(Action::Resume(wire::MissionResume {}));
    let refused = steering.admit(&mut resume, &agent()).unwrap_err();
    assert!(refused.contains("an approver held"), "{refused}");
    steering.admit(&mut resume, &human()).unwrap();
    steering.accepted(&resume, &human());
    steering.accepted(&hold, &agent());
    steering.admit(&mut resume, &agent()).unwrap();
}

#[test]
fn only_an_approver_states_that_a_member_is_down() {
    let steering = Steering::default();
    let down = || {
        intent(Action::MemberDown(wire::MissionMemberDown {
            member: "SIM-3".to_owned(),
        }))
    };
    let refused = steering.admit(&mut down(), &agent());
    assert!(refused.is_err_and(|reason| reason.contains("approver secret")));
    assert!(steering.admit(&mut down(), &human()).is_ok());
}
