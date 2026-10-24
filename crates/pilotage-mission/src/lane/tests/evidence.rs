//! A verdict names its run, task, attempt and picture: one for another
//! run, a picture never sent, or a second verdict changes nothing. A
//! line's evidence is pending until every sent picture is answered, and a
//! line nobody judged is not usable.

use pilotage_protocol::wire;

use super::*;
use crate::lane::{FrameTicket, VerdictRefused};

fn along(lane: &mut Lane, line: usize) {
    lane.on_progress(&RunProgress {
        ..toward(2 * line - 1, usize::MAX / 4)
    });
}

fn evidence(lane: &Lane, task: &str) -> (wire::MissionTaskState, wire::MissionEvidenceState) {
    let status = lane.status();
    let task = status
        .tasks
        .iter()
        .find(|t| t.task_id == task)
        .expect("task");
    let evidence = task.evidence.clone().unwrap_or_default();
    (
        wire::MissionTaskState::try_from(task.state).unwrap(),
        wire::MissionEvidenceState::try_from(evidence.state).unwrap(),
    )
}

fn usable() -> Option<FrameAssessment> {
    Some(FrameAssessment {
        usable: true,
        hazard: false,
    })
}

/// Three pictures of line 1, each sent for judgment.
fn sent(lane: &mut Lane) -> Vec<FrameTicket> {
    along(lane, 1);
    let tickets: Vec<FrameTicket> = (0..3).filter_map(|_| lane.on_frame()).collect();
    for ticket in &tickets {
        lane.on_sent(ticket);
    }
    tickets
}

#[test]
fn a_verdict_for_another_run_or_a_second_verdict_changes_nothing() {
    let (mut lane, _) = running_lane();
    let tickets = sent(&mut lane);
    let mut stale = tickets[0].clone();
    stale.run_id = "an-earlier-run".to_owned();
    assert!(matches!(
        lane.on_assessment(&stale, usable()),
        Err(VerdictRefused::OtherRun { .. })
    ));
    lane.on_assessment(&tickets[0], usable()).unwrap();
    assert!(matches!(
        lane.on_assessment(&tickets[0], usable()),
        Err(VerdictRefused::Duplicate { .. })
    ));
    let mut unsent = tickets[0].clone();
    unsent.frame = 99;
    assert!(matches!(
        lane.on_assessment(&unsent, usable()),
        Err(VerdictRefused::NotSent { .. })
    ));
    let status = lane.status();
    let line = status
        .tasks
        .iter()
        .find(|t| t.task_id == "line-01")
        .unwrap();
    assert_eq!(line.evidence.as_ref().map(|e| e.assessed), Some(1));
}

#[test]
fn a_line_settles_when_its_late_verdicts_arrive_after_the_next_line() {
    let (mut lane, _) = running_lane();
    let tickets = sent(&mut lane);
    along(&mut lane, 2);
    assert_eq!(
        evidence(&lane, "line-01"),
        (
            wire::MissionTaskState::Incomplete,
            wire::MissionEvidenceState::Pending
        ),
        "flown, but its verdicts are still out"
    );
    for ticket in &tickets[..2] {
        lane.on_assessment(ticket, usable()).unwrap();
    }
    assert_eq!(
        evidence(&lane, "line-01").1,
        wire::MissionEvidenceState::Pending
    );
    // The last picture was not judged: two usable pictures still meet the need.
    lane.on_assessment(&tickets[2], None).unwrap();
    assert_eq!(
        evidence(&lane, "line-01"),
        (
            wire::MissionTaskState::Done,
            wire::MissionEvidenceState::Met
        )
    );
}

#[test]
fn on_a_judged_lane_a_line_nobody_judged_is_not_done() {
    let (mut lane, _) = running_lane();
    along(&mut lane, 1);
    for _ in 0..3 {
        lane.on_frame();
    }
    along(&mut lane, 2);
    assert_eq!(
        evidence(&lane, "line-01"),
        (
            wire::MissionTaskState::Incomplete,
            wire::MissionEvidenceState::NotJudged
        )
    );
}

#[test]
fn without_a_judge_a_line_reads_as_not_judged() {
    let mut unjudged = config();
    unjudged.judged = false;
    let (mut lane, _) = running_lane_with(unjudged);
    along(&mut lane, 1);
    for _ in 0..3 {
        lane.on_frame();
    }
    along(&mut lane, 2);
    assert_eq!(
        evidence(&lane, "line-01"),
        (
            wire::MissionTaskState::Done,
            wire::MissionEvidenceState::NotJudged
        ),
        "captured, and said to be unjudged rather than usable"
    );
    let status = lane.status();
    let line = status
        .tasks
        .iter()
        .find(|t| t.task_id == "line-01")
        .unwrap();
    assert_eq!(line.evidence.as_ref().map(|e| e.usable), Some(0));
}

#[test]
fn a_line_wants_judgments_until_its_need_is_sent() {
    let (mut lane, _) = running_lane();
    along(&mut lane, 1);
    let first = lane.on_frame().expect("line 1");
    assert!(lane.wants_judgment(&first));
    lane.on_sent(&first);
    let second = lane.on_frame().expect("line 1");
    assert!(
        lane.wants_judgment(&second),
        "the plan needs two usable pictures"
    );
    lane.on_sent(&second);
    let third = lane.on_frame().expect("line 1");
    assert!(!lane.wants_judgment(&third), "two answers are awaited");
    // One comes back unusable: the line needs another picture judged.
    lane.on_assessment(
        &first,
        Some(FrameAssessment {
            usable: false,
            hazard: false,
        }),
    )
    .unwrap();
    assert!(lane.wants_judgment(&third));
    lane.on_sent(&third);
    lane.on_assessment(&second, usable()).unwrap();
    lane.on_assessment(&third, usable()).unwrap();
    let fourth = lane.on_frame().expect("line 1");
    assert!(!lane.wants_judgment(&fourth), "the period paces the rest");
}
