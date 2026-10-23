use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::{ApprovalPolicy, DeskRefusal, IntentKind, MissionCommand, MissionDesk, SurveyFile};

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
    // A proposal another client made is approved only under a stated ceiling.
    let unseen = MissionCommand::Approve {
        digest: Some("abce".to_owned()),
        policy: ApprovalPolicy {
            max_height_m: Some(120.0),
            ..ApprovalPolicy::default()
        },
    };
    let Ok(Action::Approve(approval)) = empty.resolve(&unseen, 1_000) else {
        panic!("an approval");
    };
    assert_eq!(approval.proposal_digest, b"\xab\xce");
    assert_eq!(approval.envelope.unwrap().max_height_m, 120.0);
}

#[test]
fn start_names_the_lanes_proposal_and_offers_follow_the_state() {
    let mut desk = proposed(b"\x01");
    let policy = ApprovalPolicy::default();
    let offered = desk.offered(true, &policy);
    assert_eq!(offered.approve, Some(70.0));
    assert!(offered.propose && !offered.start && !offered.hold);
    assert_eq!(desk.offered(false, &policy), super::Offered::default());

    desk.on_status(status(wire::MissionLaneState::Admitted, b"\x01"));
    let start = desk.resolve(&MissionCommand::Start { digest: None }, 0);
    assert_eq!(
        start,
        Ok(Action::Start(wire::MissionStart {
            proposal_digest: vec![1]
        }))
    );
    desk.on_status(status(wire::MissionLaneState::Running, b"\x01"));
    let view = desk.view(true, &policy).unwrap();
    assert_eq!(view.state, "running");
    assert_eq!(view.offered, vec![IntentKind::Hold, IntentKind::Abort]);
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["offered"], serde_json::json!(["hold", "abort"]));
    assert_eq!(json["proposal"]["digest"], "01");
}

#[test]
fn a_receipt_for_an_intent_this_desk_did_not_send_changes_nothing() {
    let mut desk = proposed(b"\x01");
    let stray = wire::MissionReceipt {
        request_id: "someone-else-1".to_owned(),
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        ..wire::MissionReceipt::default()
    };
    assert!(desk.on_receipt(stray).is_none());
    assert!(desk.receipt().unwrap().accepted());
}
