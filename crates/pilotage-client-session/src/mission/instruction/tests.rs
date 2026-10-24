use serde_json::{Value, json};

use super::{Chain, ChainLane, InstructionError, Offers};
use crate::mission::{MissionCommand, Reassign};

const RUNNING: Offers = Offers {
    hold: true,
    resume: false,
    revise: true,
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

/// A chat response whose message is `reply`.
fn response(reply: &Value) -> Value {
    json!({"choices": [{"message": {"content": reply.to_string()}}]})
}

fn reply(action: &str, lane: &str, from: &str, to: &str, fraction: f64) -> Value {
    json!({"understood": "restated", "action": action, "lane": lane,
           "from": from, "to": to, "fraction": fraction})
}

#[test]
fn the_request_names_the_chain_the_offered_actions_and_the_selection() {
    let request = chain().request("move these to SIM-B1", &["SIM-B2".to_owned()]);
    let prompt = request["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("- BATTLEFIELD leads SIM-B1, SIM-B2"));
    assert!(prompt.contains("selected: SIM-B2"));
    let lanes = &request["response_format"]["json_schema"]["schema"]["properties"]["lane"]["enum"];
    assert!(lanes.as_array().unwrap().iter().any(|l| l == "SIM-B2"));
    assert!(
        !prompt.contains("- resume"),
        "nothing is held, so nothing resumes"
    );
    let schema = &request["response_format"]["json_schema"]["schema"];
    let actions: Vec<&str> = schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(actions, ["hold", "finish", "move_lines", "unable"]);
    for forbidden in ["approve", "start", "abort"] {
        assert!(!request.to_string().contains(&format!("\"{forbidden}\"")));
    }
}

#[test]
fn a_move_becomes_a_drafted_reassignment_that_waits_for_review() {
    let parsed = chain()
        .parse(
            "move all of them",
            &response(&reply("move_lines", "BATTLEFIELD", "SIM-B2", "SIM-B1", 1.0)),
        )
        .unwrap();
    assert_eq!(parsed.lane, "BATTLEFIELD");
    assert_eq!(
        parsed.describe(),
        "draft for BATTLEFIELD: move 100 % of SIM-B2's unflown lines to SIM-B1; it waits for approval"
    );
    assert_eq!(
        parsed.command,
        MissionCommand::Revise {
            objective: None,
            reassign: Some(Reassign {
                from: "SIM-B2".to_owned(),
                to: "SIM-B1".to_owned(),
                fraction: 1.0,
            }),
            finish: false,
            within_envelope: false,
        }
    );
    let finish = chain()
        .parse(
            "move all of them",
            &response(&reply("finish", "CAMPUS", "", "", 0.0)),
        )
        .unwrap();
    assert!(matches!(
        finish.command,
        MissionCommand::Revise {
            finish: true,
            within_envelope: false,
            ..
        }
    ));
}

#[test]
fn a_reply_the_chain_cannot_take_is_refused_with_its_reason() {
    let c = chain();
    let refused = |r: Value| c.parse("move all of them", &response(&r)).unwrap_err();
    assert_eq!(
        refused(reply("unable", "COMMAND", "", "", 0.0)),
        InstructionError::Unable("restated".to_owned())
    );
    assert!(matches!(
        refused(reply("hold", "SEMINARY", "", "", 0.0)),
        InstructionError::UnknownLane(_)
    ));
    assert!(matches!(
        refused(reply("resume", "COMMAND", "", "", 0.0)),
        InstructionError::NotOffered { .. }
    ));
    assert!(matches!(
        refused(reply("approve", "COMMAND", "", "", 0.0)),
        InstructionError::NotOffered { .. }
    ));
    // A member the chain names but this client does not follow is refused,
    // never sent to the lead above it.
    assert_eq!(
        refused(reply("finish", "SIM-B2", "", "", 0.0)),
        InstructionError::UnknownLane("SIM-B2".to_owned())
    );
    // Lines move only between members the lane leads.
    assert!(matches!(
        refused(reply("move_lines", "COMMAND", "SIM-B2", "SIM-B1", 0.5)),
        InstructionError::Members { .. }
    ));
    assert!(matches!(
        refused(reply("move_lines", "COMMAND", "CAMPUS", "CAMPUS", 0.5)),
        InstructionError::Members { .. }
    ));
    assert_eq!(
        refused(reply("move_lines", "COMMAND", "CAMPUS", "BATTLEFIELD", 0.0)),
        InstructionError::Fraction(0.0)
    );
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
    let r = response(&reply(
        "move_lines",
        "COMMAND",
        "CAMPUS",
        "BATTLEFIELD",
        0.25,
    ));
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
