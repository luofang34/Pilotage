//! A client proposes a report and answers the question its run asks, as the
//! desk last saw the question.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::{DeskRefusal, MissionCommand, MissionDesk, status};

fn asking(questions: &[(&str, &[u8])]) -> MissionDesk {
    let mut desk = MissionDesk::new("test", 7);
    let mut running = status(wire::MissionLaneState::Running, b"r");
    running.tasks = questions
        .iter()
        .map(|(id, digest)| wire::MissionTaskStatus {
            task_id: (*id).to_owned(),
            question: Some(wire::MissionQuestion {
                operation_id: format!("run-1/{id}/1"),
                prompt: "Is this right?".to_owned(),
                subject: "21.5 C".to_owned(),
                subject_digest: digest.to_vec(),
            }),
            ..wire::MissionTaskStatus::default()
        })
        .collect();
    desk.on_status(running);
    desk
}

#[test]
fn an_answer_names_the_operation_and_binds_the_subject_the_desk_saw() {
    let desk = asking(&[("confirm", b"seen")]);
    assert_eq!(desk.questions().len(), 1);
    let command = MissionCommand::Answer {
        operation: None,
        accept: true,
    };
    let Ok(Action::Answer(answer)) = desk.resolve(&command, 0) else {
        panic!("an answer");
    };
    assert_eq!(answer.operation_id, "run-1/confirm/1");
    assert_eq!(answer.subject_digest, b"seen");
    assert!(answer.accepted);
    assert!(answer.answered_by.is_none(), "the host writes who answered");
}

#[test]
fn an_answer_needs_a_question_and_names_one_when_several_are_asked() {
    let none = asking(&[]);
    let any = MissionCommand::Answer {
        operation: None,
        accept: true,
    };
    assert!(matches!(
        none.resolve(&any, 0),
        Err(DeskRefusal::NoQuestion(_))
    ));
    let two = asking(&[("a", b"1"), ("b", b"2")]);
    assert!(matches!(
        two.resolve(&any, 0),
        Err(DeskRefusal::NoQuestion(_))
    ));
    let named = MissionCommand::Answer {
        operation: Some("run-1/b/1".to_owned()),
        accept: false,
    };
    let Ok(Action::Answer(answer)) = two.resolve(&named, 0) else {
        panic!("an answer");
    };
    assert_eq!(answer.subject_digest, b"2");
}

#[test]
fn one_command_proposes_an_objective_of_any_kind() {
    let desk = MissionDesk::new("test", 7);
    let report = wire::MissionObjective {
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Report(
            wire::ReportObjective {
                source: "temperature".to_owned(),
                question: "Is this right?".to_owned(),
            },
        )),
        ..wire::MissionObjective::default()
    };
    let check = wire::MissionObjective {
        objective_id: "check-1".to_owned(),
        revision: 3,
        kind: Some(wire::mission_objective::Kind::Check(wire::CheckObjective {
            source: "temperature".to_owned(),
            limit: "30".to_owned(),
        })),
        ..wire::MissionObjective::default()
    };
    for objective in [report, check] {
        // The command travels as JSON, as a client writes it on a line.
        let line = serde_json::to_string(&MissionCommand::propose_objective(&objective)).unwrap();
        assert!(line.contains(r#""command":"propose_objective""#), "{line}");
        let command: MissionCommand = serde_json::from_str(&line).unwrap();
        let Ok(Action::Propose(proposed)) = desk.resolve(&command, 5) else {
            panic!("a proposal");
        };
        assert_eq!(proposed.kind, objective.kind, "the kind arrives whole");
        if objective.objective_id.is_empty() {
            assert_eq!(proposed.objective_id, "test-objective-5");
            assert_eq!(proposed.revision, 1);
        } else {
            assert_eq!(proposed.objective_id, objective.objective_id);
            assert_eq!(proposed.revision, objective.revision);
        }
    }
}

#[test]
fn what_the_objective_states_passes_through_and_an_unstated_origin_is_the_clients() {
    let desk = MissionDesk::new("test", 7);
    let stated = wire::RecordOrigin {
        issuer: "planner".to_owned(),
        subject: "cell-2".to_owned(),
        organization: "survey".to_owned(),
    };
    let objective = wire::MissionObjective {
        objective_id: "check-1".to_owned(),
        origin: Some(stated.clone()),
        revision: 2,
        previous_digest: vec![7; 32],
        delegated_by: "TEAM".to_owned(),
        kind: Some(wire::mission_objective::Kind::Check(wire::CheckObjective {
            source: "temperature".to_owned(),
            limit: "30".to_owned(),
        })),
        ..wire::MissionObjective::default()
    };
    let Ok(Action::Propose(proposed)) =
        desk.resolve(&MissionCommand::propose_objective(&objective), 5)
    else {
        panic!("a proposal");
    };
    assert_eq!(proposed, objective, "nothing the objective states changes");
    let unstated = wire::MissionObjective {
        origin: None,
        ..objective
    };
    let Ok(Action::Propose(proposed)) =
        desk.resolve(&MissionCommand::propose_objective(&unstated), 5)
    else {
        panic!("a proposal");
    };
    let origin = proposed.origin.expect("an origin");
    assert_eq!(
        (origin.issuer.as_str(), origin.subject.as_str()),
        ("local", "test")
    );
}

#[test]
fn an_objective_that_does_not_decode_or_names_no_kind_is_refused() {
    let desk = MissionDesk::new("test", 7);
    for text in ["not hex", "ff", "", "abc"] {
        let command = MissionCommand::ProposeObjective {
            objective: text.to_owned(),
        };
        assert!(
            matches!(desk.resolve(&command, 5), Err(DeskRefusal::BadObjective(_))),
            "{text:?}"
        );
    }
}
