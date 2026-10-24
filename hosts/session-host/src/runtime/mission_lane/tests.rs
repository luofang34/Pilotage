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

#[test]
fn a_run_the_vehicle_ended_has_ended_with_its_reason() {
    let status = AutomationStatus {
        mission_state: Some(MissionState::Climb),
        ended_by_vehicle: Some("the vehicle landed outside the plan"),
        ..AutomationStatus::default()
    };
    let progress = run_progress(&status, false);
    assert!(progress.ended);
    assert_eq!(
        progress.ended_because,
        Some("the vehicle landed outside the plan")
    );
}
