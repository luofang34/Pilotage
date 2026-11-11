use pilotage_adapter_api::FcStateSample;
use pilotage_protocol::wire;

#[test]
fn the_fc_readiness_crosses_the_wire_and_its_absence_stays_absent() {
    let sample = |ready_to_arm| FcStateSample {
        landing_approach: None,
        landed_state: None,
        motor_state: None,
        arm_state: 1,
        ready_to_arm,
        last_command: None,
        stamp: None,
    };
    for ready_to_arm in [Some(true), Some(false), None] {
        let state = super::super::fc_state_to_wire(sample(ready_to_arm));
        assert_eq!(state.ready_to_arm, ready_to_arm);
        let bytes = prost::Message::encode_to_vec(&state);
        let decoded: wire::FcState = prost::Message::decode(bytes.as_slice()).expect("decodes");
        assert_eq!(decoded.ready_to_arm, ready_to_arm, "round trip");
    }
}
