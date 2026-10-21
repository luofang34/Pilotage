use prost::Message;

use crate::wire;

#[test]
fn unspecified_payload_enum_values_keep_the_zero_wire_sentinel() {
    assert_eq!(
        [
            wire::PayloadSupportState::Unspecified as i32,
            wire::PayloadHardwareSupport::Unspecified as i32,
            wire::PayloadImplementation::Unspecified as i32,
            wire::PayloadStopGuarantee::Unspecified as i32,
            wire::PayloadReferenceFrame::Unspecified as i32,
            wire::ZoomUnit::Unspecified as i32,
            wire::PayloadCommandStage::Unspecified as i32,
            wire::PayloadConnectionRole::Unspecified as i32,
            wire::PayloadObservationSource::Unspecified as i32,
            wire::PayloadEndReason::Unspecified as i32,
        ],
        [0; 10]
    );
}

#[test]
fn absent_payload_fields_stay_unknown_and_distinct_from_a_measured_zero() {
    let empty = &[][..];
    let capabilities = wire::PayloadCapabilities::decode(empty).unwrap();
    assert_eq!(capabilities.connection_role, 0);
    assert!(capabilities.gimbal.is_none() && capabilities.zoom.is_none());
    let axis_capabilities = wire::PayloadAxisCapabilities::decode(empty).unwrap();
    assert!(axis_capabilities.mechanics.is_none());
    let mechanics = wire::PayloadAxisMechanics::decode(empty).unwrap();
    assert_eq!((mechanics.movement, mechanics.stabilization), (0, 0));
    let state = wire::PayloadState::decode(empty).unwrap();
    assert!(state.pitch.is_none() && state.roll.is_none() && state.yaw.is_none());
    assert!(state.zoom.is_none() && state.command.is_none());
    let progress = wire::PayloadCommandProgress::decode(empty).unwrap();
    assert_eq!(
        (progress.stage, progress.end_reason, progress.frame),
        (0, 0, 0)
    );
    assert!(progress.pitch_target_rad.is_none() && progress.observed_complete_at.is_none());
    let mut axis = wire::PayloadAxisObservation::decode(empty).unwrap();
    assert_eq!((axis.frame, axis.source), (0, 0));
    assert!(axis.angle_rad.is_none() && axis.rate_rad_s.is_none());
    assert!(axis.sampled_at.is_none() && axis.received_at.is_none());
    assert!(axis.encode_to_vec().is_empty());
    axis.angle_rad = Some(0.0);
    let bytes = axis.encode_to_vec();
    assert!(!bytes.is_empty());
    let decoded = wire::PayloadAxisObservation::decode(bytes.as_slice()).unwrap();
    assert_eq!(decoded.angle_rad, Some(0.0));
    assert!(decoded.rate_rad_s.is_none());
    assert_eq!((decoded.frame, decoded.source), (0, 0));
}
