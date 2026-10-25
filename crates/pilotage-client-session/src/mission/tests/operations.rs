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
fn a_report_command_proposes_a_report_objective() {
    let desk = MissionDesk::new("test", 7);
    let command: MissionCommand = serde_json::from_str(
        r#"{"command":"propose_report","source":"temperature","question":"Is this right?"}"#,
    )
    .unwrap();
    let Ok(Action::Propose(objective)) = desk.resolve(&command, 5) else {
        panic!("a proposal");
    };
    let Some(wire::mission_objective::Kind::Report(report)) = objective.kind else {
        panic!("a report objective");
    };
    assert_eq!(report.source, "temperature");
    assert_eq!(objective.revision, 1);
}
