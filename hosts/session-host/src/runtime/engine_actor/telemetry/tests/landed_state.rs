use pilotage_adapter_api::{LandedState, LandedStateSample, MeasurementStamp};
use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;

#[test]
fn landed_observation_does_not_refresh_the_arm_observation() {
    let mut sample = super::truth_and_fc_sample();
    let fc = sample.fc_state.as_mut().expect("FC sample");
    let arm_stamp = fc.stamp;
    let landed_stamp = MeasurementStamp {
        sequence: arm_stamp.sequence.wrapping_add(9),
        acquired_at_ns: arm_stamp.acquired_at_ns + 10_000,
        ..arm_stamp
    };
    fc.landed_state = Some(LandedStateSample {
        state: LandedState::OnGround,
        stamp: landed_stamp,
    });
    let mapped = super::sample_to_wire(sample, MonoTimestamp::from_nanos(20_000));
    let fc = mapped.fc_state.expect("wire FC state");
    let arm = fc.stamp.expect("arm stamp");
    assert_eq!(arm.sequence, arm_stamp.sequence);
    assert_eq!(arm.acquired_at_ns, arm_stamp.acquired_at_ns);
    let landed = fc.landed_state.expect("wire landed report");
    assert_eq!(landed.state, wire::LandedState::OnGround as i32);
    let stamp = landed.stamp.expect("landed stamp");
    assert_eq!(stamp.sequence, landed_stamp.sequence);
    assert_eq!(stamp.acquired_at_ns, landed_stamp.acquired_at_ns);
    assert_eq!(stamp.source_incarnation, arm.source_incarnation);
    assert_eq!(stamp.source_epoch, arm.source_epoch);
}

#[test]
fn missing_and_unknown_landed_states_do_not_assert_on_ground() {
    let sample = super::truth_and_fc_sample();
    let mapped = super::sample_to_wire(sample.clone(), MonoTimestamp::from_nanos(1));
    assert!(mapped.fc_state.expect("FC state").landed_state.is_none());

    let mut unknown = sample;
    let fc = unknown.fc_state.as_mut().expect("FC sample");
    fc.landed_state = Some(LandedStateSample {
        state: LandedState::Unknown,
        stamp: fc.stamp,
    });
    let mapped = super::sample_to_wire(unknown, MonoTimestamp::from_nanos(2));
    let landed = mapped
        .fc_state
        .expect("FC state")
        .landed_state
        .expect("unknown observation");
    assert_eq!(landed.state, wire::LandedState::Unspecified as i32);
    assert!(landed.stamp.is_some());
}
