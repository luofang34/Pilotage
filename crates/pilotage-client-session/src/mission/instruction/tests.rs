use serde_json::{Value, json};

use super::catalog::{Effect, ParamKind, Precondition};
use super::{Chain, ChainLane, Drafted, InstructionError, Offers};
use crate::mission::{MissionCommand, Reassign};

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
    assert_eq!(
        chain()
            .parse("let battlefield help campus", &r)
            .unwrap_err(),
        InstructionError::NeedsAmount
    );
    for words in [
        "move 25% of campus",
        "a quarter of campus",
        "give all of it",
        "the remaining lines",
    ] {
        assert!(chain().parse(words, &r).is_ok(), "{words}");
    }
}
