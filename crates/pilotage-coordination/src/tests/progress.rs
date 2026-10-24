//! Survey progress as a member's mission core reports it, for driving lanes in tests.

use pilotage_mission::{Lane, RunProgress};
use pilotage_protocol::wire;

/// What `lane`'s mission core reports while the vehicle flies toward waypoint `waypoint`
/// of the survey plan: line `k` is flown toward waypoint `2k + 1` after its transit toward
/// `2k`, and the way home follows the last line. The phases carry the lane's own task
/// identities.
pub(super) fn toward(lane: &Lane, waypoint: usize) -> RunProgress {
    let status = lane.status();
    let is_line = |id: &String| {
        status
            .tasks
            .iter()
            .any(|t| &t.task_id == id && t.kind == wire::MissionTaskKind::SurveyLine as i32)
    };
    let ids = lane.task_ids();
    let line_ids: Vec<String> = ids.iter().filter(|id| is_line(id)).cloned().collect();
    let lines = line_ids.len();
    if waypoint >= 2 * lines {
        return RunProgress {
            active_task: ids.iter().find(|id| !is_line(id)).cloned(),
            finished_tasks: line_ids,
            ..RunProgress::default()
        };
    }
    let flown = waypoint / 2;
    RunProgress {
        active_task: line_ids.get(flown).map(|id| {
            if waypoint % 2 == 1 {
                id.clone()
            } else {
                format!("transit:{id}")
            }
        }),
        finished_tasks: line_ids[..flown].to_vec(),
        ..RunProgress::default()
    }
}

/// `status` with every task named in other words when `renamed`: names are for display,
/// and nothing a team decides may follow them.
pub(super) fn renamed(mut status: wire::MissionStatus, renamed: bool) -> wire::MissionStatus {
    if renamed {
        for (index, task) in status.tasks.iter_mut().enumerate() {
            task.name = format!("Aufgabe {index}");
        }
        status.active_task = "unterwegs".to_owned();
    }
    status
}
