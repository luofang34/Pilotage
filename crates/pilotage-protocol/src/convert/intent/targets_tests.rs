//! Neither target may ride an action that does not name it.

#![allow(clippy::expect_used, clippy::panic)]

use crate::wire;

use super::action_from_wire;

/// Neither target may ride an action that does not name it.
///
/// A stray target is a sender and a receiver disagreeing about what was
/// asked for. Reading past one would let a mode request arrive carrying a
/// control law, or a feel request carrying a flight mode, and the receiver
/// would act on the half it understood.
#[test]
fn a_stray_target_of_the_other_kind_is_refused() {
    let mode_with_feel = wire::ControlActionRequest {
        action: wire::ControlAction::ModeRequest as i32,
        mode_target: wire::ModeTarget::Hold as i32,
        action_id: 7,
        feel_target: wire::FeelTarget::Agile as i32,
    };
    assert!(action_from_wire(mode_with_feel).is_err());

    let feel_with_mode = wire::ControlActionRequest {
        action: wire::ControlAction::FeelModeRequest as i32,
        mode_target: wire::ModeTarget::Hold as i32,
        action_id: 7,
        feel_target: wire::FeelTarget::Agile as i32,
    };
    assert!(action_from_wire(feel_with_mode).is_err());

    // Each is accepted when it carries only its own target.
    let mode = wire::ControlActionRequest {
        action: wire::ControlAction::ModeRequest as i32,
        mode_target: wire::ModeTarget::Hold as i32,
        action_id: 7,
        feel_target: wire::FeelTarget::Unspecified as i32,
    };
    assert!(action_from_wire(mode).is_ok());

    let feel = wire::ControlActionRequest {
        action: wire::ControlAction::FeelModeRequest as i32,
        mode_target: wire::ModeTarget::Unspecified as i32,
        action_id: 7,
        feel_target: wire::FeelTarget::Agile as i32,
    };
    assert!(action_from_wire(feel).is_ok());

    // A feel request with no target names no law, and the receiver refuses
    // rather than guessing which one to install.
    let untargeted = wire::ControlActionRequest {
        action: wire::ControlAction::FeelModeRequest as i32,
        mode_target: wire::ModeTarget::Unspecified as i32,
        action_id: 7,
        feel_target: wire::FeelTarget::Unspecified as i32,
    };
    assert!(action_from_wire(untargeted).is_err());
}
