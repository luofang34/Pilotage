use prost::Message;

use super::MotorState;
use crate::wire;

#[test]
fn motor_reports_survive_encoding_without_an_arm_report() {
    for state in [
        MotorState::Unknown,
        MotorState::Stopped,
        MotorState::Running,
    ] {
        let fc = wire::FcState {
            motor_state: Some(wire::MotorStateObservation {
                state: state.to_wire() as i32,
                stamp: Some(wire::MeasurementStamp {
                    sequence: 7,
                    acquired_at_ns: 900,
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };
        let bytes = fc.encode_to_vec();
        let decoded = wire::FcState::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.arm_state, 0);
        assert!(decoded.stamp.is_none());
        let motor = decoded.motor_state.unwrap();
        assert_eq!(MotorState::from_wire(motor.state), state);
        let stamp = motor.stamp.unwrap();
        assert_eq!(stamp.sequence, 7);
        assert_eq!(stamp.acquired_at_ns, 900);
    }
}

#[test]
fn absent_or_unknown_motor_reports_do_not_assert_stopped() {
    let legacy = wire::FcState::decode([0x08_u8, 0x01].as_slice()).unwrap();
    assert_eq!(legacy.arm_state, 1);
    assert!(legacy.motor_state.is_none());
    for raw in [0, -1, 42, i32::MAX] {
        let observation = wire::MotorStateObservation {
            state: raw,
            stamp: None,
        };
        let bytes = observation.encode_to_vec();
        let decoded = wire::MotorStateObservation::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.state, raw);
        assert_eq!(MotorState::from_wire(decoded.state), MotorState::Unknown);
        assert!(decoded.stamp.is_none());
    }
}
