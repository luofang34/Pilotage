use std::io::Write as _;

use pilotage_client_session::mission::{ApprovalPolicy, MissionCommand};

use super::{Action, parse};

const CERT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn args(rest: &[&str]) -> Vec<String> {
    ["--host", "https://127.0.0.1:1/pilotage", "--cert", CERT]
        .iter()
        .chain(rest)
        .map(|s| (*s).to_owned())
        .collect()
}

#[test]
fn approve_takes_the_policy_flags_over_the_shared_default() {
    let command = parse(&args(&[
        "approve",
        "abcd",
        "--max-height",
        "120",
        "--refly",
        "2",
    ]))
    .unwrap();
    let Action::Send(MissionCommand::Approve { digest, policy }) = command.action else {
        panic!("an approve");
    };
    assert_eq!(digest.as_deref(), Some("abcd"));
    assert_eq!(
        policy,
        ApprovalPolicy {
            max_height_m: Some(120.0),
            max_refly_lines: 2,
            ..ApprovalPolicy::default()
        }
    );
    assert_eq!(command.name, "mission-operator");
    assert!(command.token.is_none());
}

#[test]
fn survey_agent_reads_its_objective_and_name() {
    let dir = std::env::temp_dir().join(format!("mission-operator-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("objective.json");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        r#"{{"title": "t", "area": [[0, 0], [0, 1], [1, 1]], "launch": [0, 0, 0], "height_m": 50}}"#
    )
    .unwrap();
    let path_text = path.to_string_lossy().into_owned();
    let command = parse(&args(&["--name", "agent-1", "survey-agent", &path_text])).unwrap();
    let Action::SurveyAgent(objective) = command.action else {
        panic!("a survey agent");
    };
    assert_eq!(objective.height_m, 50.0);
    assert_eq!(command.name, "agent-1");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        parse(&args(&["survey-agent"])).is_err(),
        "the objective is required"
    );
}
