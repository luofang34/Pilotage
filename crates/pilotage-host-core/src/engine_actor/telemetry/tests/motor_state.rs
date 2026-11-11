use pilotage_adapter_api::{
    LandedState, LandedStateSample, MeasurementStamp, MotorState, MotorStateSample,
};
use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;
use prost::Message;

#[test]
fn motor_reports_keep_their_own_identity_through_host_mapping_and_encoding() {
    let mut sample = super::truth_and_fc_sample();
    let fc = sample.fc_state.as_mut().unwrap();
    let arm_stamp = fc.stamp.unwrap();
    let landed_stamp = MeasurementStamp {
        sequence: 5,
        acquired_at_ns: 100,
        ..arm_stamp
    };
    fc.landed_state = Some(LandedStateSample {
        state: LandedState::OnGround,
        stamp: landed_stamp,
    });
    let motor_stamp = MeasurementStamp {
        sequence: 8,
        acquired_at_ns: 200,
        ..arm_stamp
    };
    fc.motor_state = Some(MotorStateSample {
        state: MotorState::Stopped,
        stamp: motor_stamp,
    });
    let first = super::sample_to_wire(sample.clone(), MonoTimestamp::from_nanos(1_000));
    let later = super::sample_to_wire(sample, MonoTimestamp::from_nanos(2_000));
    assert_eq!(
        first.fc_state, later.fc_state,
        "publishing is not observation"
    );
    let bytes = first.encode_to_vec();
    let decoded = wire::TelemetrySample::decode(bytes.as_slice()).unwrap();
    let fc = decoded.fc_state.unwrap();
    let arm = fc.stamp.unwrap();
    assert_eq!(arm.sequence, arm_stamp.sequence);
    assert_eq!(arm.acquired_at_ns, arm_stamp.acquired_at_ns);
    let landed = fc.landed_state.unwrap().stamp.unwrap();
    assert_eq!(landed.sequence, landed_stamp.sequence);
    assert_eq!(landed.acquired_at_ns, landed_stamp.acquired_at_ns);
    let motor = fc.motor_state.unwrap();
    assert_eq!(MotorState::from_wire(motor.state), MotorState::Stopped);
    let stamp = motor.stamp.unwrap();
    assert_eq!(stamp.source_id, motor_stamp.source_id);
    assert_eq!(
        stamp.source_incarnation,
        motor_stamp.source_incarnation.into_bytes()
    );
    assert_eq!(stamp.source_epoch, motor_stamp.source_epoch);
    assert_eq!(stamp.sequence, motor_stamp.sequence);
    assert_eq!(stamp.acquired_at_ns, motor_stamp.acquired_at_ns);
    assert_eq!(stamp.role, wire::SourceRole::FcState as i32);
    assert_eq!(
        stamp.integrity,
        wire::SourceIntegrity::ChecksummedOnly as i32
    );
    assert_eq!(stamp.clock, wire::MeasurementClock::HostMonotonic as i32);
}

#[test]
fn arm_reports_do_not_create_motor_reports() {
    for arm_state in [0, 1, 2] {
        let mut sample = super::truth_and_fc_sample();
        sample.fc_state.as_mut().unwrap().arm_state = arm_state;
        let mapped = super::sample_to_wire(sample, MonoTimestamp::from_nanos(1));
        assert!(mapped.fc_state.unwrap().motor_state.is_none());
    }
}

#[test]
fn a_motor_only_publication_does_not_create_arm_provenance() {
    let mut sample = super::truth_and_fc_sample();
    let fc = sample.fc_state.as_mut().unwrap();
    let motor_stamp = fc.stamp.take().unwrap();
    fc.arm_state = 0;
    fc.motor_state = Some(MotorStateSample {
        state: MotorState::Stopped,
        stamp: motor_stamp,
    });
    let mapped = super::sample_to_wire(sample, MonoTimestamp::from_nanos(1_000));
    let bytes = mapped.encode_to_vec();
    let decoded = wire::TelemetrySample::decode(bytes.as_slice()).unwrap();
    let fc = decoded.fc_state.unwrap();
    assert_eq!(fc.arm_state, 0);
    assert!(fc.stamp.is_none());
    assert!(fc.landed_state.is_none());
    assert_eq!(fc.last_command_kind, 0);
    let motor = fc.motor_state.unwrap();
    assert_eq!(MotorState::from_wire(motor.state), MotorState::Stopped);
    let stamp = motor.stamp.unwrap();
    assert_eq!(
        stamp.source_incarnation,
        motor_stamp.source_incarnation.into_bytes()
    );
    assert_eq!(stamp.source_id, motor_stamp.source_id);
    assert_eq!(stamp.source_epoch, motor_stamp.source_epoch);
    assert_eq!(stamp.sequence, motor_stamp.sequence);
    assert_eq!(stamp.acquired_at_ns, motor_stamp.acquired_at_ns);
}

#[test]
fn unknown_and_running_motor_reports_do_not_become_stopped() {
    for state in [MotorState::Unknown, MotorState::Running] {
        let mut sample = super::truth_and_fc_sample();
        let fc = sample.fc_state.as_mut().unwrap();
        fc.motor_state = Some(MotorStateSample {
            state,
            stamp: fc.stamp.unwrap(),
        });
        let mapped = super::sample_to_wire(sample, MonoTimestamp::from_nanos(1));
        let bytes = mapped.encode_to_vec();
        let decoded = wire::TelemetrySample::decode(bytes.as_slice()).unwrap();
        let motor = decoded.fc_state.unwrap().motor_state.unwrap();
        assert_eq!(MotorState::from_wire(motor.state), state);
        assert!(motor.stamp.is_some());
    }
}
