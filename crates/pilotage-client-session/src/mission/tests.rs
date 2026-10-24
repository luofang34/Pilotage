use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::{ApprovalPolicy, DeskRefusal, MissionCommand, MissionDesk, SurveyFile};

/// The objective with every field named.
const FILE: &str = r#"{"title": "Battlefield to Nassau Hall",
    "area": [[40.333, -74.672], [40.333, -74.665], [40.3366, -74.665], [40.3366, -74.672]],
    "launch": [40.3328, -74.6722, 40.0], "height_m": 60, "cruise_mps": 12, "sidelap": 0.6,
    "fov_deg": 69.6, "min_usable_frames_per_line": 2, "line_bearing_deg": 0}"#;

fn survey() -> SurveyFile {
    SurveyFile::parse(FILE).unwrap()
}

fn proposal(digest: &[u8]) -> wire::MissionProposal {
    wire::MissionProposal {
        digest: digest.to_vec(),
        objective: Some(survey().objective("test", 1)),
        ..wire::MissionProposal::default()
    }
}

fn status(state: wire::MissionLaneState, digest: &[u8]) -> Box<wire::MissionStatus> {
    Box::new(wire::MissionStatus {
        lane: "COMMAND".to_owned(),
        state: state as i32,
        proposal_digest: digest.to_vec(),
        ..wire::MissionStatus::default()
    })
}

/// A desk holding an accepted proposal `digest`, as if its propose was
/// answered.
fn proposed(digest: &[u8]) -> MissionDesk {
    let mut desk = MissionDesk::new("test", 7);
    let mut engine = crate::tests::engine();
    let refused = desk.send(&engine, Action::Hold(wire::MissionHold {}));
    assert_eq!(refused, Err(DeskRefusal::NotAdmitted("Hold")));
    crate::tests::admit(&mut engine, 1, 1);
    let objective = survey().objective("test", 1);
    let (request_id, actions) = desk.send(&engine, Action::Propose(objective)).unwrap();
    assert_eq!(request_id, "test-7-1");
    assert_eq!(actions.len(), 1);
    desk.on_receipt(wire::MissionReceipt {
        request_id,
        outcome: wire::MissionReceiptOutcome::Accepted as i32,
        proposal: Some(proposal(digest)),
        ..wire::MissionReceipt::default()
    })
    .unwrap();
    desk.on_status(status(wire::MissionLaneState::Proposed, digest));
    desk
}

#[test]
fn the_survey_file_becomes_a_first_revision_objective() {
    let objective = survey().objective("sokoly-app", 1_700_000_000_000);
    assert_eq!(objective.objective_id, "objective-1700000000000");
    assert_eq!(objective.revision, 1);
    assert_eq!(objective.origin.unwrap().subject, "sokoly-app");
    let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind else {
        panic!("a survey");
    };
    assert_eq!(survey.area.len(), 4);
    assert_eq!(survey.launch.unwrap().latitude_deg, 40.3328);
    assert_eq!(survey.launch_altitude_m, 40.0);
    assert_eq!(survey.across_track_fov_deg, 69.6);
    assert_eq!(survey.line_bearing_deg, Some(0.0));
    assert!(survey.return_to_launch);
    let short = r#"{"title": "t", "area": [[0, 0], [0, 1], [1, 1]], "launch": [0, 0, 0]}"#;
    assert_eq!(SurveyFile::parse(short).unwrap().height_m, 0.0);
    let unknown = r#"{"title": "t", "area": [], "launch": [0, 0, 0], "altitude": 3}"#;
    assert!(SurveyFile::parse(unknown).is_err());
}

#[test]
fn commands_read_from_one_json_line_each() {
    let line = format!(r#"{{"command": "propose", "objective": {FILE}}}"#);
    let command: MissionCommand = serde_json::from_str(&line).unwrap();
    assert_eq!(
        command,
        MissionCommand::Propose {
            objective: survey()
        }
    );
    let command: MissionCommand =
        serde_json::from_str(r#"{"command": "approve", "policy": {"max_refly_lines": 2}}"#)
            .unwrap();
    let MissionCommand::Approve { digest, policy } = command else {
        panic!("an approve");
    };
    assert_eq!(digest, None);
    assert_eq!(policy.max_refly_lines, 2);
    assert_eq!(
        policy.height_margin_m,
        ApprovalPolicy::default().height_margin_m
    );
    for line in [r#"{"command": "hold"}"#, r#"{"command": "abort"}"#] {
        serde_json::from_str::<MissionCommand>(line).unwrap();
    }
    assert!(serde_json::from_str::<MissionCommand>(r#"{"command": "land"}"#).is_err());
    assert!(serde_json::from_str::<MissionCommand>(r#"{"command": "hold", "now": 1}"#).is_err());
}

#[test]
fn approve_takes_the_held_proposal_under_the_default_policy() {
    let desk = proposed(b"\xab\xcd");
    let command = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    let Ok(Action::Approve(approval)) = desk.resolve(&command, 1_000) else {
        panic!("an approval");
    };
    assert_eq!(approval.proposal_digest, b"\xab\xcd");
    assert!(approval.approver.is_none(), "the host names the approver");
    let envelope = approval.envelope.unwrap();
    assert_eq!(envelope.max_height_m, 70.0);
    assert_eq!(envelope.expires_unix_ms, 1_000 + 3_600_000);
    assert_eq!(envelope.max_refly_lines, 4);
    assert!(envelope.permit_revise && !envelope.permit_change_objective);

    let named = MissionCommand::Approve {
        digest: Some("abce".to_owned()),
        policy: ApprovalPolicy::default(),
    };
    assert_eq!(
        desk.resolve(&named, 1_000),
        Err(DeskRefusal::UnknownDigest("abce".to_owned()))
    );
    let empty = MissionDesk::new("test", 7);
    let refused = empty.resolve(&command, 1_000).unwrap_err();
    assert_eq!(refused, DeskRefusal::NoProposal);
    // A proposal this client has not seen is never approved, whatever ceiling
    // the policy states.
    let unseen = MissionCommand::Approve {
        digest: Some("abce".to_owned()),
        policy: ApprovalPolicy {
            max_height_m: Some(120.0),
            ..ApprovalPolicy::default()
        },
    };
    assert_eq!(
        empty.resolve(&unseen, 1_000),
        Err(DeskRefusal::UnknownDigest("abce".to_owned()))
    );
}

/// Sends a fetch through `desk` and files the lane's answer, the proposal
/// `digest`.
fn fetched(desk: &mut MissionDesk, digest: &[u8]) {
    let mut engine = crate::tests::engine();
    crate::tests::admit(&mut engine, 1, 1);
    let Ok(action) = desk.resolve(&MissionCommand::Fetch {}, 0) else {
        panic!("a fetch");
    };
    assert_eq!(
        action,
        Action::Fetch(wire::MissionFetch {
            digest: digest.to_vec()
        })
    );
    let (request_id, _) = desk.send(&engine, action).unwrap();
    desk.on_receipt(wire::MissionReceipt {
        request_id,
        outcome: wire::MissionReceiptOutcome::Accepted as i32,
        proposal: Some(proposal(digest)),
        ..wire::MissionReceipt::default()
    })
    .unwrap();
}

#[test]
fn a_proposal_another_client_made_is_approved_only_once_fetched() {
    let mut desk = MissionDesk::new("person", 7);
    let mut named = status(wire::MissionLaneState::Proposed, b"\x42");
    named.height_m = 110.0;
    desk.on_status(named);
    let command = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    // The lane's summary does not stand for the plan.
    assert_eq!(desk.offered(true, &ApprovalPolicy::default()).approve, None);
    assert_eq!(desk.resolve(&command, 0), Err(DeskRefusal::NotHeld));
    assert_eq!(desk.unseen_proposal(), Some(b"\x42".to_vec()));
    fetched(&mut desk, b"\x42");
    assert_eq!(desk.unseen_proposal(), None);
    assert_eq!(
        desk.offered(true, &ApprovalPolicy::default()).approve,
        Some(70.0),
        "sized by the fetched plan's own height"
    );
    let Ok(Action::Approve(approval)) = desk.resolve(&command, 0) else {
        panic!("an approval");
    };
    assert_eq!(approval.proposal_digest, b"\x42");
}

#[test]
fn a_revision_continues_the_lanes_plan_and_its_draft_is_what_approve_names() {
    let mut desk = MissionDesk::new("agent", 7);
    let mut running = status(wire::MissionLaneState::Running, b"\x01");
    running.run_id = "run-1".to_owned();
    running.height_m = 110.0;
    desk.on_status(running.clone());
    let line = r#"{"command": "revise", "reassign": {"from": "CAMPUS", "to": "BATTLEFIELD", "fraction": 0.5}}"#;
    let command: MissionCommand = serde_json::from_str(line).unwrap();
    let Ok(Action::Revise(revise)) = desk.resolve(&command, 0) else {
        panic!("a revision");
    };
    assert_eq!(revise.base_digest, b"\x01");
    assert_eq!(revise.run_id, "run-1");
    assert!(!revise.within_envelope);
    let Some(wire::mission_revise::Change::Reassign(reassign)) = revise.change else {
        panic!("a reassignment");
    };
    assert_eq!(
        (reassign.from_member.as_str(), reassign.fraction),
        ("CAMPUS", 0.5)
    );
    let both = r#"{"command": "revise", "finish": true, "reassign": {"from": "a", "to": "b", "fraction": 1}}"#;
    let both: MissionCommand = serde_json::from_str(both).unwrap();
    assert_eq!(desk.resolve(&both, 0), Err(DeskRefusal::RevisionShape));

    // A drafted revision is what an approval names, once this client holds it.
    running.successor = Some(wire::MissionSuccessor {
        digest: b"\x02".to_vec(),
        state: wire::MissionSuccessorState::Drafted as i32,
        ..wire::MissionSuccessor::default()
    });
    desk.on_status(running);
    let approve = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    assert_eq!(desk.resolve(&approve, 0), Err(DeskRefusal::NotHeld));
    fetched(&mut desk, b"\x02");
    assert_eq!(
        desk.offered(true, &ApprovalPolicy::default()).approve,
        Some(70.0)
    );
    let Ok(Action::Approve(approval)) = desk.resolve(&approve, 0) else {
        panic!("an approval");
    };
    assert_eq!(approval.proposal_digest, b"\x02");
}

#[test]
fn a_proposal_is_approvable_as_soon_as_its_receipt_arrives_before_any_status() {
    let mut desk = MissionDesk::new("test", 7);
    let mut engine = crate::tests::engine();
    crate::tests::admit(&mut engine, 1, 1);
    let objective = survey().objective("test", 1);
    let (request_id, _) = desk.send(&engine, Action::Propose(objective)).unwrap();
    desk.on_receipt(wire::MissionReceipt {
        request_id,
        outcome: wire::MissionReceiptOutcome::Accepted as i32,
        proposal: Some(proposal(&[7; 32])),
        ..wire::MissionReceipt::default()
    })
    .unwrap();
    assert!(desk.status().is_none(), "no status has come yet");
    let command = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    assert!(matches!(desk.resolve(&command, 1), Ok(Action::Approve(_))));
}

#[test]
fn a_client_renews_the_approval_of_a_held_run_it_never_saw_by_fetching_its_plan() {
    let mut desk = MissionDesk::new("person", 7);
    desk.on_status(status(wire::MissionLaneState::Held, b"\x51"));
    let approve = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    // No proposal is under review, but the plan in progress may be approved again.
    assert_eq!(desk.unseen_proposal(), None);
    assert_eq!(desk.unseen_approvable(), Some(b"\x51".to_vec()));
    assert!(desk.resolve(&approve, 0).is_err());
    fetched(&mut desk, b"\x51");
    let Ok(Action::Approve(approval)) = desk.resolve(&approve, 0) else {
        panic!("a renewal");
    };
    assert_eq!(approval.proposal_digest, b"\x51");
}

#[test]
fn an_expired_approval_offers_a_renewal_of_the_run_and_no_resume() {
    let mut desk = MissionDesk::new("person", 7);
    let mut held = status(wire::MissionLaneState::Held, b"\x52");
    held.run_id = "run-7".to_owned();
    held.holds = vec![wire::MissionHoldReason::ApprovalExpired as i32];
    desk.on_status(held.clone());
    assert_eq!(
        desk.unseen_proposal(),
        Some(b"\x52".to_vec()),
        "the plan to renew is fetched"
    );
    fetched(&mut desk, b"\x52");
    let offered = desk.offered(true, &ApprovalPolicy::default());
    assert!(
        offered.approve.is_some(),
        "the plan in progress may be renewed"
    );
    assert!(
        !offered.resume,
        "a resume is refused while the expiry holds the run"
    );
    let approve = MissionCommand::Approve {
        digest: None,
        policy: ApprovalPolicy::default(),
    };
    let Ok(Action::Approve(renewal)) = desk.resolve(&approve, 0) else {
        panic!("a renewal");
    };
    assert_eq!(renewal.run_id, "run-7", "the renewal names its run");

    held.holds = vec![wire::MissionHoldReason::Operator as i32];
    desk.on_status(held);
    let offered = desk.offered(true, &ApprovalPolicy::default());
    assert!(offered.resume && offered.approve.is_none());
}
