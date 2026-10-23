use pilotage_mission::MissionState;

use super::run_progress;
use crate::runtime::AutomationStatus;

#[test]
fn a_run_whose_mission_failed_has_ended() {
    let mut status = AutomationStatus {
        mission_state: Some(MissionState::Enroute),
        ..AutomationStatus::default()
    };
    assert!(!run_progress(&status, false).ended);
    status.mission_state = Some(MissionState::Failed);
    assert!(run_progress(&status, false).ended);
}
