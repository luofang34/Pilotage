use prost::Message;

use super::TakeoffStartState;
use crate::wire;

#[test]
fn declared_start_states_survive_capability_encoding() {
    for state in [
        TakeoffStartState::ArmedOnGround,
        TakeoffStartState::MotorsStoppedOnGround,
    ] {
        let capability = wire::ActionCapability {
            action: wire::ControlAction::Takeoff as i32,
            takeoff_start_state: state.to_wire() as i32,
            ..Default::default()
        };
        let bytes = capability.encode_to_vec();
        let decoded = wire::ActionCapability::decode(bytes.as_slice()).unwrap();
        assert_eq!(
            TakeoffStartState::from_wire(decoded.takeoff_start_state),
            Some(state)
        );
    }
}

#[test]
fn absent_and_unknown_start_states_do_not_supply_a_requirement() {
    let legacy = wire::ActionCapability::decode([0x08_u8, 0x0a].as_slice()).unwrap();
    assert_eq!(legacy.action, wire::ControlAction::Takeoff as i32);
    assert_eq!(
        TakeoffStartState::from_wire(legacy.takeoff_start_state),
        None
    );
    assert_eq!(
        TakeoffStartState::option_to_wire(None),
        wire::TakeoffStartState::Unspecified
    );
    for raw in [0, -1, 42, i32::MAX] {
        let capability = wire::ActionCapability {
            action: wire::ControlAction::Takeoff as i32,
            takeoff_start_state: raw,
            ..Default::default()
        };
        let bytes = capability.encode_to_vec();
        let decoded = wire::ActionCapability::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.takeoff_start_state, raw);
        assert_eq!(
            TakeoffStartState::from_wire(decoded.takeoff_start_state),
            None
        );
    }
}
