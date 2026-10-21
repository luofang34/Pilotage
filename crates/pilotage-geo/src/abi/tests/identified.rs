use super::*;
use indicate_frames::{ClockId, FrameId, FrameKind};

#[test]
fn encoding_preserves_instances_and_does_not_merge_equal_clock_domains() {
    let mut input = raw();
    input.attitude.body_frame = FrameId::new(FrameKind::Body, 121).expect("body");
    input.attitude.navigation_frame = FrameId::new(FrameKind::Ned, 122).expect("navigation");
    input.reference_time.clock = ClockId::new(ClockDomain::Gnss, 123).expect("other receiver");
    let validated = input.validate(&SIM).expect("well formed identities");
    let decoded = decode_frame(&encode_frame(&validated), &SIM).expect("decoded identities");
    assert_eq!(decoded.attitude().body_frame, input.attitude.body_frame);
    assert_eq!(
        decoded.attitude().navigation_frame,
        input.attitude.navigation_frame
    );
    assert_eq!(decoded.reference_time().clock, input.reference_time.clock);
    assert_eq!(
        decoded.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence)
    );
}

#[test]
fn an_unidentified_wire_clock_is_refused_instead_of_becoming_local_time() {
    let mut bytes = encode_frame(&validated());
    let instance_offset = SVS_FRAME_LEN - crate::abi::TIMESTAMP_LEN + 1;
    bytes[instance_offset..instance_offset + 16].fill(0);
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::Identity { field: "clock", .. })
    ));
}

#[test]
fn a_camera_cannot_replace_aircraft_attitude_in_the_raw_or_wire_contract() {
    let mut input = raw();
    input.attitude.body_frame = FrameId::new(FrameKind::CameraFrd, 42).expect("camera");
    assert!(matches!(
        input.validate(&SIM),
        Err(crate::GeoError::AttitudeFrames { .. })
    ));
    let inputs = crate::derive_inputs(
        &input.position,
        &input.attitude,
        &input.external,
        input.reference_time,
        &SIM,
    );
    assert_eq!(inputs.attitude, InputHealth::Failed);
    let mut bytes = encode_frame(&validated());
    let frame_offset = 4 + crate::abi::POSITION_LEN + crate::abi::QUAT_LEN;
    bytes[frame_offset] = FrameKind::CameraFrd.to_u8();
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::Identity {
            field: "attitude_frame",
            ..
        })
    ));
}
