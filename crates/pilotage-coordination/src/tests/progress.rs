//! Survey progress as a member's mission core reports it, for driving lanes in tests.

use pilotage_mission::RunProgress;

/// What the survey's phases report while the vehicle flies toward waypoint `waypoint` of a
/// plan of `lines` lines: line `k` is flown toward waypoint `2k + 1` after its transit
/// toward `2k`, and the way home follows the last line.
pub(super) fn toward(waypoint: usize, lines: usize) -> RunProgress {
    let line = |number: usize| format!("line-{number:02}");
    if waypoint >= 2 * lines {
        return RunProgress {
            active_task: Some("home".to_owned()),
            finished_tasks: (1..=lines).map(line).collect(),
            ..RunProgress::default()
        };
    }
    let flown = waypoint / 2;
    RunProgress {
        active_task: Some(if waypoint % 2 == 1 {
            line(flown + 1)
        } else {
            format!("transit-{:02}", flown + 1)
        }),
        finished_tasks: (1..=flown).map(line).collect(),
        ..RunProgress::default()
    }
}
