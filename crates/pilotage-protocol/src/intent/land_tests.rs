use super::{ActionKind, ControlAction, ModeTarget, actions_conflict};

#[test]
fn land_is_a_distinct_procedure_with_no_implicit_flight_transition() {
    assert_eq!(ControlAction::Land.kind(), ActionKind::Land);
    for other in [
        ControlAction::Arm,
        ControlAction::Disarm,
        ControlAction::Takeoff,
        ControlAction::Land,
        ControlAction::ModeRequest {
            target: ModeTarget::Hold,
        },
        ControlAction::ModeRequest {
            target: ModeTarget::Return,
        },
    ] {
        assert!(actions_conflict(&[ControlAction::Land, other]));
        assert!(actions_conflict(&[other, ControlAction::Land]));
    }
    assert!(!actions_conflict(&[ControlAction::Land]));
    assert!(!actions_conflict(&[
        ControlAction::Land,
        ControlAction::GimbalRecenter,
    ]));
}
