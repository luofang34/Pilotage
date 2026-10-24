//! A task's execution outcome, its evidence and its judgment are separate
//! facts: a verdict that arrives after the execution ended changes the
//! evidence and the judgment, never how the execution ended.

use pilotage_protocol::wire;
use wire::MissionTaskExecution as Execution;
use wire::MissionTaskJudgment as Judgment;

use super::*;

fn facts(lane: &Lane, name: &str) -> (Execution, Judgment, wire::MissionTaskState) {
    let status = lane.status();
    let task = status.tasks.iter().find(|t| t.name == name).expect("task");
    (
        Execution::try_from(task.execution).unwrap(),
        Judgment::try_from(task.judgment).unwrap(),
        wire::MissionTaskState::try_from(task.state).unwrap(),
    )
}

fn usable() -> Option<FrameAssessment> {
    Some(FrameAssessment {
        usable: true,
        hazard: false,
    })
}

#[test]
fn verdicts_after_an_abort_do_not_make_the_cut_line_done() {
    use wire::mission_intent::Action;
    let (mut lane, _) = running_lane();
    lane.on_progress(&toward(&lane, 1));
    let tickets: Vec<_> = (0..3).filter_map(|_| lane.on_frame()).collect();
    for ticket in &tickets {
        lane.on_sent(ticket);
    }
    lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    for ticket in &tickets {
        lane.on_assessment(ticket, usable()).unwrap();
    }
    assert_eq!(
        facts(&lane, "line-01"),
        (
            Execution::Cancelled,
            Judgment::NotMet,
            wire::MissionTaskState::Incomplete
        ),
        "enough usable pictures, but the line was cut short"
    );
    let status = lane.status();
    let line = status.tasks.iter().find(|t| t.name == "line-01").unwrap();
    let evidence = line.evidence.clone().unwrap_or_default();
    assert_eq!(evidence.usable, 3, "the late verdicts are still counted");
    assert_eq!(
        evidence.collection,
        wire::MissionEvidenceCollection::Settled as i32
    );
}

#[test]
fn an_unplanned_end_on_the_way_home_fails_the_way_home() {
    let (mut lane, lines) = running_lane();
    lane.on_progress(&toward(&lane, 2 * lines + 1));
    lane.on_progress(&RunProgress {
        ended: true,
        ended_because: Some("the vehicle landed outside the plan"),
        ..toward(&lane, 2 * lines + 1)
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Failed);
    let status = lane.status();
    let home = status
        .tasks
        .iter()
        .find(|t| t.kind == wire::MissionTaskKind::ReturnHome as i32)
        .expect("the way home");
    assert_eq!(home.execution, Execution::Failed as i32);
    assert_eq!(home.judgment, Judgment::NotMet as i32);
    assert_ne!(home.state, wire::MissionTaskState::Done as i32);
}

#[test]
fn a_completed_line_is_undetermined_until_its_verdicts_are_in() {
    let (mut lane, _) = running_lane();
    lane.on_progress(&toward(&lane, 1));
    let tickets: Vec<_> = (0..2).filter_map(|_| lane.on_frame()).collect();
    for ticket in &tickets {
        lane.on_sent(ticket);
    }
    lane.on_progress(&toward(&lane, 3));
    assert_eq!(
        facts(&lane, "line-01"),
        (
            Execution::Completed,
            Judgment::Undetermined,
            wire::MissionTaskState::Incomplete
        )
    );
    for ticket in &tickets {
        lane.on_assessment(ticket, usable()).unwrap();
    }
    assert_eq!(
        facts(&lane, "line-01"),
        (
            Execution::Completed,
            Judgment::Met,
            wire::MissionTaskState::Done
        )
    );
}
