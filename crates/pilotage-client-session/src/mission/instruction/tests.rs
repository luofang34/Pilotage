use pilotage_protocol::wire;
use serde_json::{Value, json};
use wire::mission_intent::Action;

use super::catalog::{Effect, ParamKind, Precondition};
use super::{Chain, ChainLane, Dispatch, Drafted, InstructionError, Offers, Progress};
use crate::mission::{MissionCommand, MissionDesk, Reassign};

const RUNNING: Offers = Offers {
    hold: true,
    resume: false,
    revise: true,
    propose: false,
};
const IDLE: Offers = Offers {
    hold: false,
    resume: false,
    revise: false,
    propose: true,
};

fn lane(name: &str, members: &[&str], offers: Offers) -> ChainLane {
    ChainLane {
        lane: name.to_owned(),
        members: members.iter().map(|m| (*m).to_owned()).collect(),
        offers,
    }
}

fn chain() -> Chain {
    Chain::new(vec![
        lane("COMMAND", &["BATTLEFIELD", "CAMPUS"], RUNNING),
        lane("BATTLEFIELD", &["SIM-B1", "SIM-B2"], RUNNING),
        lane("CAMPUS", &["SIM-C1", "SIM-C2"], RUNNING),
    ])
}

/// One step of a reply, every catalog field filled.
fn step(action: &str, lane: &str) -> Value {
    json!({"action": action, "lane": lane, "from": "", "to": "", "fraction": 0, "area": ""})
}

/// A chat response holding `steps`.
fn response(steps: &[Value]) -> Value {
    let reply = json!({"understood": "restated", "steps": steps});
    json!({"choices": [{"message": {"content": reply.to_string()}}]})
}

fn moved(lane: &str, from: &str, to: &str, fraction: f64) -> Value {
    let mut s = step("move_lines", lane);
    s["from"] = json!(from);
    s["to"] = json!(to);
    s["fraction"] = json!(fraction);
    s
}

#[test]
fn the_request_comes_from_the_catalog_and_the_lanes() {
    let request = chain().request("move these to SIM-B1", &["SIM-B2".to_owned()]);
    let prompt = request["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("- BATTLEFIELD leads SIM-B1, SIM-B2; offers hold, finish, move_lines"));
    assert!(prompt.contains("- survey_area: plan a new survey"));
    assert!(prompt.contains("selected: SIM-B2"));
    let schema = &request["response_format"]["json_schema"]["schema"];
    let item = &schema["properties"]["steps"]["items"];
    let actions: Vec<&str> = item["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        actions,
        [
            "hold",
            "resume",
            "finish",
            "move_lines",
            "survey_area",
            "unable"
        ]
    );
    for field in ["from", "to", "fraction", "area"] {
        assert!(item["properties"].get(field).is_some(), "{field}");
    }
    for forbidden in ["approve", "start", "abort"] {
        assert!(!request.to_string().contains(&format!("\"{forbidden}\"")));
    }
}

#[test]
fn the_catalog_describes_itself_in_the_form_lanes_may_publish() {
    let chain = chain();
    let survey = chain
        .actions()
        .iter()
        .find(|a| a.name == "survey_area")
        .unwrap();
    assert_eq!(survey.offered, Precondition::Idle);
    assert_eq!(survey.effect, Effect::Draft);
    assert_eq!(survey.params[0].kind, ParamKind::Area);
    let published = serde_json::to_value(survey).unwrap();
    assert_eq!(published["params"][0]["kind"], "area");
    assert_eq!(published["offered"], "idle");
}

#[test]
fn one_instruction_becomes_its_steps_in_order() {
    let reply = response(&[
        step("hold", "BATTLEFIELD"),
        moved("COMMAND", "CAMPUS", "BATTLEFIELD", 0.5),
    ]);
    let parsed = chain()
        .parse(
            "hold battlefield, then give half of campus's lines to it",
            &reply,
        )
        .unwrap();
    let lanes: Vec<&str> = parsed.steps.iter().map(|s| s.lane.as_str()).collect();
    assert_eq!(lanes, ["BATTLEFIELD", "COMMAND"]);
    assert_eq!(parsed.steps[0].command(), Some(&MissionCommand::Hold {}));
    assert_eq!(
        parsed.steps[1].command(),
        Some(&MissionCommand::Revise {
            objective: None,
            reassign: Some(Reassign {
                from: "CAMPUS".to_owned(),
                to: "BATTLEFIELD".to_owned(),
                fraction: 0.5,
            }),
            finish: false,
            within_envelope: false,
        })
    );
    assert!(parsed.describe().contains(
        "BATTLEFIELD holds in place and keeps its plan; then draft for COMMAND: move 50 %"
    ));
}

#[test]
fn a_survey_of_a_named_area_waits_for_its_area_and_for_an_idle_lane() {
    let chain = Chain::new(vec![
        lane("COMMAND", &["BATTLEFIELD"], IDLE),
        lane("BATTLEFIELD", &["SIM-B1"], RUNNING),
    ]);
    let mut survey = step("survey_area", "COMMAND");
    survey["area"] = json!(" Faculty Road ");
    let parsed = chain
        .parse(
            "plan and survey the Faculty Road area",
            &response(&[survey.clone()]),
        )
        .unwrap();
    assert_eq!(
        parsed.steps[0].drafted,
        Drafted::Survey {
            area: "Faculty Road".to_owned()
        }
    );
    assert_eq!(
        parsed.steps[0].command(),
        None,
        "nothing to send before the area is found"
    );
    assert!(parsed.describe().contains("found on the map first"));
    survey["lane"] = json!("BATTLEFIELD");
    assert_eq!(
        chain
            .parse(
                "survey Faculty Road with battlefield",
                &response(&[survey.clone()])
            )
            .unwrap_err(),
        InstructionError::Busy("BATTLEFIELD".to_owned())
    );
    survey["area"] = json!("");
    survey["lane"] = json!("COMMAND");
    assert_eq!(
        chain
            .parse("survey somewhere", &response(&[survey]))
            .unwrap_err(),
        InstructionError::NeedsArea
    );
}

#[test]
fn a_step_the_chain_cannot_take_refuses_the_whole_instruction() {
    let c = chain();
    let refused = |steps: &[Value]| c.parse("move half of them", &response(steps)).unwrap_err();
    assert_eq!(
        refused(&[step("unable", "COMMAND")]),
        InstructionError::Unable("restated".to_owned())
    );
    assert!(matches!(
        refused(&[step("hold", "SEMINARY")]),
        InstructionError::UnknownLane(_)
    ));
    // A member the chain names but this client does not follow is refused, never sent
    // to the lead above it.
    assert_eq!(
        refused(&[step("finish", "SIM-B2")]),
        InstructionError::UnknownLane("SIM-B2".to_owned())
    );
    assert!(matches!(
        refused(&[step("resume", "COMMAND")]),
        InstructionError::NotOffered { .. }
    ));
    assert!(matches!(
        refused(&[step("approve", "COMMAND")]),
        InstructionError::NotOffered { .. }
    ));
    assert!(matches!(
        refused(&[moved("COMMAND", "SIM-B2", "SIM-B1", 0.5)]),
        InstructionError::Members { .. }
    ));
    assert_eq!(
        refused(&[moved("COMMAND", "CAMPUS", "BATTLEFIELD", 0.0)]),
        InstructionError::Fraction(0.0)
    );
    // A good first step does not carry a bad second one through.
    assert!(matches!(
        refused(&[step("hold", "COMMAND"), step("hold", "SEMINARY")]),
        InstructionError::UnknownLane(_)
    ));
    assert!(matches!(refused(&[]), InstructionError::Malformed(_)));
    assert!(matches!(
        c.parse(
            "hold",
            &json!({"choices": [{"message": {"content": "hold it"}}]})
        ),
        Err(InstructionError::Malformed(_))
    ));
}

#[test]
fn a_move_whose_words_state_no_amount_asks_for_one() {
    let r = response(&[moved("COMMAND", "CAMPUS", "BATTLEFIELD", 0.25)]);
    // A lane's number is not an amount, and neither is a count of lines.
    for words in [
        "let battlefield help campus",
        "move 3 of SIM-B2's lines to SIM-B1",
    ] {
        assert_eq!(
            chain().parse(words, &r).unwrap_err(),
            InstructionError::NeedsAmount,
            "{words}"
        );
    }
}

#[test]
fn each_word_form_states_its_own_amount_and_no_other() {
    let parse = |words: &str, fraction: f64| {
        chain().parse(
            words,
            &response(&[moved("COMMAND", "CAMPUS", "BATTLEFIELD", fraction)]),
        )
    };
    for (words, fraction) in [
        ("move all of campus to battlefield", 1.0),
        ("give battlefield the remaining lines of campus", 1.0),
        ("move half of campus", 0.5),
        ("a third of campus", 0.333),
        ("two thirds of campus", 0.667),
        ("a quarter of campus", 0.25),
        ("three quarters of campus", 0.75),
        ("move 25% of campus", 0.25),
        ("move 40 percent of campus", 0.4),
        ("move 0.3 of campus.", 0.3),
        ("move 1/5 of campus", 0.2),
    ] {
        assert!(parse(words, fraction).is_ok(), "{words}");
        assert_eq!(
            parse(words, if fraction > 0.5 { 0.1 } else { 0.9 }).unwrap_err(),
            InstructionError::UnsureAmount {
                from: "CAMPUS".to_owned(),
                fraction: if fraction > 0.5 { 0.1 } else { 0.9 },
            },
            "{words}"
        );
    }
}

#[test]
fn two_amounts_each_stand_only_for_the_member_they_name() {
    let c = chain();
    let words = "move half of SIM-B1 and a quarter of SIM-B2 to the other";
    let moves = |b1: f64, b2: f64| {
        response(&[
            moved("BATTLEFIELD", "SIM-B1", "SIM-B2", b1),
            moved("BATTLEFIELD", "SIM-B2", "SIM-B1", b2),
        ])
    };
    assert!(c.parse(words, &moves(0.5, 0.25)).is_ok());
    assert_eq!(
        c.parse(words, &moves(0.25, 0.5)).unwrap_err(),
        InstructionError::UnsureAmount {
            from: "SIM-B1".to_owned(),
            fraction: 0.25,
        },
        "a swapped pair"
    );
    // Words that tie neither amount to a member ask again.
    assert!(matches!(
        c.parse("move half and a quarter of them", &moves(0.5, 0.25)),
        Err(InstructionError::UnsureAmount { .. })
    ));
}

/// A dispatch of a hold and then a resume for COMMAND, its desk, and an admitted engine.
fn dispatched() -> (Dispatch, MissionDesk, crate::ClientEngine) {
    let mut engine = crate::tests::engine();
    crate::tests::admit(&mut engine, 1, 1);
    let dispatch = Dispatch::new(vec![
        ("COMMAND".to_owned(), MissionCommand::Hold {}),
        ("COMMAND".to_owned(), MissionCommand::Resume {}),
    ]);
    (dispatch, MissionDesk::new("test", 7), engine)
}

fn answered(desk: &mut MissionDesk, request_id: String, outcome: wire::MissionReceiptOutcome) {
    desk.on_receipt(wire::MissionReceipt {
        request_id,
        outcome: outcome as i32,
        reason: "the lane is not running".to_owned(),
        ..wire::MissionReceipt::default()
    })
    .unwrap();
}

#[test]
fn a_step_is_sent_only_after_the_step_before_it_is_accepted() {
    let (mut dispatch, mut desk, engine) = dispatched();
    assert_eq!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send {
            lane: "COMMAND".to_owned(),
            command: MissionCommand::Hold {}
        }
    );
    assert_eq!(dispatch.poll(|_| Some(&desk), 10), Progress::Waiting);
    let (hold, _) = desk
        .send(&engine, Action::Hold(wire::MissionHold {}))
        .unwrap();
    assert_eq!(dispatch.poll(|_| Some(&desk), 20), Progress::Waiting);
    answered(&mut desk, hold, wire::MissionReceiptOutcome::Accepted);
    assert_eq!(
        dispatch.poll(|_| Some(&desk), 30),
        Progress::Send {
            lane: "COMMAND".to_owned(),
            command: MissionCommand::Resume {}
        }
    );
    let (resume, _) = desk
        .send(&engine, Action::Resume(wire::MissionResume {}))
        .unwrap();
    answered(&mut desk, resume, wire::MissionReceiptOutcome::Accepted);
    assert_eq!(dispatch.poll(|_| Some(&desk), 40), Progress::Done);
}

#[test]
fn a_refused_step_stops_the_steps_after_it() {
    let (mut dispatch, mut desk, engine) = dispatched();
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send { .. }
    ));
    let (hold, _) = desk
        .send(&engine, Action::Hold(wire::MissionHold {}))
        .unwrap();
    answered(&mut desk, hold, wire::MissionReceiptOutcome::Refused);
    assert_eq!(
        dispatch.poll(|_| Some(&desk), 10),
        Progress::Stopped {
            lane: "COMMAND".to_owned(),
            reason: "refused: the lane is not running".to_owned()
        }
    );
    assert_eq!(dispatch.remaining(), 0, "the resume is never sent");
    assert_eq!(dispatch.poll(|_| Some(&desk), 20), Progress::Done);
}

#[test]
fn a_step_the_desk_does_not_send_or_the_lane_does_not_answer_stops_the_rest() {
    let (mut dispatch, desk, _) = dispatched();
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send { .. }
    ));
    assert_eq!(dispatch.poll(|_| Some(&desk), 5_000), Progress::Waiting);
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 5_001),
        Progress::Stopped { .. }
    ));
    let (mut dispatch, mut desk, engine) = dispatched();
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send { .. }
    ));
    desk.send(&engine, Action::Hold(wire::MissionHold {}))
        .unwrap();
    assert_eq!(dispatch.poll(|_| Some(&desk), 60_000), Progress::Waiting);
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 60_001),
        Progress::Stopped { .. }
    ));
    assert!(matches!(
        Dispatch::new(vec![("GONE".to_owned(), MissionCommand::Hold {})]).poll(|_| None, 0),
        Progress::Stopped { .. }
    ));
}

#[test]
fn another_command_on_the_same_desk_answers_nothing_for_a_step() {
    let (mut dispatch, mut desk, engine) = dispatched();
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send { .. }
    ));
    let (hold, _) = desk
        .send(&engine, Action::Hold(wire::MissionHold {}))
        .unwrap();
    // The person presses Abort in the panel while the hold waits for its answer.
    let (abort, _) = desk
        .send(&engine, Action::Abort(wire::MissionAbort {}))
        .unwrap();
    answered(&mut desk, abort, wire::MissionReceiptOutcome::Accepted);
    assert_eq!(dispatch.poll(|_| Some(&desk), 10), Progress::Waiting);
    answered(&mut desk, hold, wire::MissionReceiptOutcome::Refused);
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 20),
        Progress::Stopped { .. }
    ));
    // A command that takes the step's request id, because the desk refused the step,
    // does not stand for the step either.
    let (mut dispatch, mut desk, engine) = dispatched();
    assert!(matches!(
        dispatch.poll(|_| Some(&desk), 0),
        Progress::Send { .. }
    ));
    let (abort, _) = desk
        .send(&engine, Action::Abort(wire::MissionAbort {}))
        .unwrap();
    answered(&mut desk, abort, wire::MissionReceiptOutcome::Accepted);
    assert_eq!(
        dispatch.poll(|_| Some(&desk), 10),
        Progress::Stopped {
            lane: "COMMAND".to_owned(),
            reason: "the desk did not send the step; another command went first".to_owned()
        }
    );
}
