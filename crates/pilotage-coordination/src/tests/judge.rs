//! The judge's verdicts steer the team mid-run: a line flown without
//! enough usable pictures is flown again by another member that has
//! finished, once the verdict settled, while the line's own member flies
//! on; the owner flies it only when nobody else is left to.

use pilotage_protocol::wire;

use super::*;

/// Flies `member`'s lines `from..to` (from 1) with three judged pictures
/// each, unusable on the lines in `bad`; completes the plan when `to`
/// passes the last line. Returns the team's intents.
fn fly_lines(
    fleet: &mut Fleet,
    member: &str,
    (from, to): (usize, usize),
    bad: &[usize],
) -> Vec<MemberIntent> {
    let lane = fleet.members.get_mut(member).expect("member");
    let lines = lane
        .status()
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .count();
    for line in from..=to.min(lines) {
        lane.on_progress(&RunProgress {
            target_waypoint: Some(2 * line - 1),
            ..RunProgress::default()
        });
        for _ in 0..3 {
            if let Some(task) = lane.on_frame() {
                lane.on_assessment(
                    &task,
                    FrameAssessment {
                        usable: !bad.contains(&line),
                        hazard: false,
                    },
                );
            }
        }
    }
    lane.on_progress(&RunProgress {
        target_waypoint: Some(2 * to.min(lines)),
        plan_complete: to >= lines,
        ended: false,
    });
    match lane.take_changed() {
        Some(status) => fleet.team.on_member_status(&status, &context("fly")),
        None => Vec::new(),
    }
}

#[test]
fn a_finished_member_flies_a_bad_line_again_while_its_owner_flies_on() {
    let (mut fleet, _) = super::revision::running_fleet(false);
    // SIM-1 finishes its plan first.
    let none = fly_lines(&mut fleet, "SIM-1", (1, 99), &[]);
    assert!(none.is_empty(), "nothing to fly again yet");
    // SIM-2's line 1 is judged unusable; the verdict settles once line 2 is flown.
    let early = fly_lines(&mut fleet, "SIM-2", (1, 1), &[1]);
    assert!(
        early.is_empty(),
        "a verdict is not acted on before it settled"
    );
    let decided = fly_lines(&mut fleet, "SIM-2", (2, 2), &[1]);
    assert_eq!(decided.len(), 1, "{}", fleet.team.status().detail);
    assert_eq!(decided[0].member, "SIM-1", "the finished member flies it");
    assert!(
        fleet.team.status().detail.contains("SIM-1 has finished"),
        "{}",
        fleet.team.status().detail
    );
    fleet.route(decided);
    assert_eq!(
        fleet.members.get("SIM-1").map(Lane::state),
        Some(wire::MissionLaneState::Running)
    );
    assert_eq!(
        fleet.members.get("SIM-2").map(Lane::state),
        Some(wire::MissionLaneState::Running)
    );

    // SIM-1 flies it with usable pictures; SIM-2's line takes that verdict.
    let after = fly_lines(&mut fleet, "SIM-1", (1, 99), &[]);
    fleet.route(after);
    let line = fleet
        .team
        .status()
        .tasks
        .into_iter()
        .find(|t| t.task_id == "SIM-2/line-01")
        .expect("SIM-2's line 1");
    assert_eq!(line.state, wire::MissionTaskState::Done as i32);
    assert_eq!(line.reflown, 1);

    // SIM-2 keeps reporting its own, older verdict on that line.
    let sim2 = fleet.members.get("SIM-2").map(Lane::status).expect("SIM-2");
    fleet.team.on_member_status(&sim2, &context("again"));
    let line = fleet
        .team
        .status()
        .tasks
        .into_iter()
        .find(|t| t.task_id == "SIM-2/line-01")
        .expect("SIM-2's line 1");
    assert_eq!(
        line.state,
        wire::MissionTaskState::Done as i32,
        "the re-flight's verdict stands"
    );
}

#[test]
fn of_several_finished_members_one_other_than_the_owner_takes_it() {
    let (mut fleet, _) = super::revision::running_fleet(false);
    for member in ["SIM-1", "SIM-3"] {
        let started = fly_lines(&mut fleet, member, (1, 99), &[]);
        assert!(started.is_empty());
    }
    // Both others are idle: one of them takes it at once.
    let decided = fly_lines(&mut fleet, "SIM-2", (1, 2), &[1]);
    assert_eq!(decided.len(), 1);
    assert_ne!(decided[0].member, "SIM-2");
}
