use super::{ControlAction, ModeTarget, actions_conflict};

#[test]
fn takeoff_cannot_share_an_unordered_set_with_a_flight_transition() {
    for other in [
        ControlAction::Arm,
        ControlAction::Disarm,
        ControlAction::ModeRequest {
            target: ModeTarget::Hold,
        },
        ControlAction::ModeRequest {
            target: ModeTarget::Return,
        },
        ControlAction::Takeoff,
    ] {
        assert!(actions_conflict(&[ControlAction::Takeoff, other]));
        assert!(actions_conflict(&[other, ControlAction::Takeoff]));
    }
    assert!(!actions_conflict(&[ControlAction::Takeoff]));
    assert!(!actions_conflict(&[
        ControlAction::Takeoff,
        ControlAction::GimbalRecenter,
    ]));
}
