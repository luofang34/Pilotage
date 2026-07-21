//! Unit tests for domain<->wire conversion and envelope framing
//! (`convert.rs`). Proptest coverage for `ControlFrame` lives in
//! `proptests.rs` alongside these hand-written fixed cases.
#![allow(clippy::expect_used, clippy::panic)]

use super::{
    ConvertError, DecodeError, SCHEMA_VERSION, decode_control_frame_envelope,
    decode_envelope_length_delimited, encode_control_frame_envelope,
    encode_envelope_length_delimited,
};
use crate::control::{
    ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame,
};
use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::wire;
use pilotage_timing::MonoTimestamp;
use prost::Message;

pub(super) fn sample_frame(payload: ControlPayload) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle: VehicleId::new(2),
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(3),
        sequence: SequenceNum::new(4),
        sampled_at: MonoTimestamp::from_nanos(5),
        profile_revision: 6,
        payload,
        intent: None,
        actions: vec![],
    }
}

#[test]
fn control_frame_roundtrips_with_empty_payload() {
    let frame = sample_frame(ControlPayload::default());
    let wire_frame: wire::ControlFrame = (&frame).into();
    let back = ScopedControlFrame::try_from(wire_frame).expect("valid conversion");
    assert_eq!(back, frame);
}

#[test]
fn control_frame_roundtrips_with_full_payload() {
    let payload = ControlPayload {
        axes: vec![(LogicalAxisId::new(0), 0.5), (LogicalAxisId::new(1), -1.0)],
        edges: vec![
            (LogicalButtonId::new(2), ButtonEdge::Pressed),
            (LogicalButtonId::new(3), ButtonEdge::Released),
        ],
    };
    let frame = sample_frame(payload);
    let wire_frame: wire::ControlFrame = (&frame).into();
    let back = ScopedControlFrame::try_from(wire_frame).expect("valid conversion");
    assert_eq!(back, frame);
}

#[test]
fn control_frame_envelope_roundtrips() {
    let frame = sample_frame(ControlPayload {
        axes: vec![(LogicalAxisId::new(0), 0.25)],
        edges: vec![(LogicalButtonId::new(1), ButtonEdge::Pressed)],
    });
    let bytes = encode_control_frame_envelope(&frame);
    let back = decode_control_frame_envelope(&bytes).expect("valid envelope");
    assert_eq!(back, frame);
}

#[test]
fn envelope_roundtrips_for_authority_event_arm() {
    let event = wire::AuthorityEvent {
        event: Some(wire::authority_event::Event::ScopeLeaseGranted(
            wire::ScopeLeaseGranted {
                principal: Some(wire::PrincipalId { value: 9 }),
                vehicle: Some(wire::VehicleId { value: 2 }),
                scope: Some(wire::ScopeId {
                    value: "vehicle.motion".to_owned(),
                }),
                generation: Some(wire::Generation { value: 1 }),
                reason: "operator request".to_owned(),
                authority_class: wire::AuthorityClass::Operator as i32,
            },
        )),
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::AuthorityEvent(event.clone())),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::AuthorityEvent(event))
    );
}

#[test]
// The deprecated legacy arm lane must keep round-tripping unchanged for
// as long as the field exists on the wire.
#[allow(deprecated)]
fn envelope_roundtrips_for_telemetry_sample_arm() {
    let sample = wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 2 }),
        tick: Some(wire::SimTick { value: 42 }),
        observed_at: Some(wire::MonoTimestamp { nanos: 100 }),
        pose: Some(wire::Pose2d {
            x_m: 1.0,
            y_m: 2.0,
            heading_rad: 0.5,
        }),
        velocity: Some(wire::Velocity2d {
            linear_x_mps: 0.1,
            linear_y_mps: 0.2,
            angular_rad_s: 0.3,
        }),
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            quat_x: 0.0,
            quat_y: 0.0,
            quat_z: 0.0,
            rate_p_rad_s: 0.01,
            rate_q_rad_s: 0.02,
            rate_r_rad_s: 0.03,
            pos_n_m: 1.0,
            pos_e_m: 2.0,
            pos_d_m: -300.0,
            vel_n_mps: 10.0,
            vel_e_mps: 0.0,
            vel_d_mps: -1.0,
            valid_flags: 0b1111,
            quality: 0,
            arm_state: 2,
            attitude_stamp: Some(wire::MeasurementStamp {
                source_id: 7,
                source_incarnation: vec![0xA5; 16],
                source_epoch: 3,
                sequence: 10,
                acquired_at_ns: 1_000_000,
                clock: wire::MeasurementClock::VehicleBoot as i32,
                role: wire::SourceRole::OperationalEstimate as i32,
                integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
            }),
            kinematics_stamp: Some(wire::MeasurementStamp {
                source_id: 7,
                source_incarnation: vec![0xA5; 16],
                source_epoch: 3,
                sequence: 5,
                acquired_at_ns: 900_000,
                clock: wire::MeasurementClock::VehicleBoot as i32,
                role: wire::SourceRole::OperationalEstimate as i32,
                integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
            }),
            estimator_status_stamp: Some(wire::MeasurementStamp {
                source_id: 7,
                source_incarnation: vec![0xA5; 16],
                source_epoch: 3,
                sequence: 11,
                acquired_at_ns: 1_000_000,
                clock: wire::MeasurementClock::VehicleBoot as i32,
                role: wire::SourceRole::OperationalEstimate as i32,
                integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
            }),
        }),
        sim_truth: None,
        fc_state: None,
        gimbal: None,
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::TelemetrySample(sample.clone())),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::TelemetrySample(sample))
    );
}

#[test]
fn envelope_roundtrips_for_host_capabilities_arm() {
    let capabilities = wire::HostCapabilities {
        host_version: "0.1.0".to_owned(),
        vehicles: vec![wire::VehicleDescriptor {
            vehicle: Some(wire::VehicleId { value: 2 }),
            display_name: "rover-1".to_owned(),
            scopes: vec![wire::ScopeDescriptor {
                scope: Some(wire::ScopeId {
                    value: "vehicle.motion".to_owned(),
                }),
                display_name: "Motion".to_owned(),
                link_loss_action: wire::LinkLossAction::Stop as i32,
                intents: vec![],
                actions: vec![],
            }],
            supported_modes: vec![wire::ExecutionMode::Realtime as i32],
        }],
        supported_modes: vec![wire::ExecutionMode::Realtime as i32],
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::HostCapabilities(
            capabilities.clone(),
        )),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::HostCapabilities(capabilities))
    );
}

#[test]
fn decode_control_frame_envelope_fails_on_garbage_bytes() {
    let result = decode_control_frame_envelope(&[0xFF, 0xFF, 0xFF]);
    assert!(matches!(result, Err(DecodeError::Prost { .. })));
}

#[test]
fn decode_control_frame_envelope_fails_on_missing_payload() {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: None,
    };
    let bytes = envelope.encode_to_vec();
    let result = decode_control_frame_envelope(&bytes);
    assert!(matches!(
        result,
        Err(DecodeError::Convert(ConvertError::MissingField { .. }))
    ));
}

#[test]
fn decode_control_frame_envelope_fails_on_wrong_payload_arm() {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::HostCapabilities(
            wire::HostCapabilities::default(),
        )),
    };
    let bytes = envelope.encode_to_vec();
    let result = decode_control_frame_envelope(&bytes);
    assert!(matches!(
        result,
        Err(DecodeError::Convert(ConvertError::MissingField { .. }))
    ));
}

#[test]
fn control_frame_conversion_fails_on_missing_session() {
    let frame = sample_frame(ControlPayload::default());
    let mut wire_frame: wire::ControlFrame = (&frame).into();
    wire_frame.session = None;
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField {
            field: "session",
            ..
        })
    ));
}

#[test]
fn control_frame_conversion_fails_on_unspecified_button_edge() {
    let payload = wire::ControlPayload {
        axes: vec![],
        edges: vec![wire::ButtonEdgeSample {
            button_id: 1,
            edge: wire::ButtonEdge::Unspecified as i32,
        }],
    };
    let mut wire_frame: wire::ControlFrame = (&sample_frame(ControlPayload::default())).into();
    wire_frame.payload = Some(payload);
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(result, Err(ConvertError::UnknownEnum { .. })));
}

#[test]
fn control_frame_conversion_fails_on_out_of_range_axis_id() {
    let payload = wire::ControlPayload {
        axes: vec![wire::AxisSample {
            axis_id: u32::from(u16::MAX) + 1,
            value: 0.0,
        }],
        edges: vec![],
    };
    let mut wire_frame: wire::ControlFrame = (&sample_frame(ControlPayload::default())).into();
    wire_frame.payload = Some(payload);
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(
        result,
        Err(ConvertError::IdOutOfRange {
            field: "axis_id",
            ..
        })
    ));
}

#[test]
fn control_frame_conversion_fails_on_out_of_range_button_id() {
    let payload = wire::ControlPayload {
        axes: vec![],
        edges: vec![wire::ButtonEdgeSample {
            button_id: u32::from(u16::MAX) + 1,
            edge: wire::ButtonEdge::Pressed as i32,
        }],
    };
    let mut wire_frame: wire::ControlFrame = (&sample_frame(ControlPayload::default())).into();
    wire_frame.payload = Some(payload);
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(
        result,
        Err(ConvertError::IdOutOfRange {
            field: "button_id",
            ..
        })
    ));
}

#[test]
fn control_frame_conversion_fails_on_non_finite_axis_value() {
    let payload = wire::ControlPayload {
        axes: vec![wire::AxisSample {
            axis_id: 0,
            value: f32::NAN,
        }],
        edges: vec![],
    };
    let mut wire_frame: wire::ControlFrame = (&sample_frame(ControlPayload::default())).into();
    wire_frame.payload = Some(payload);
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(
        result,
        Err(ConvertError::NonFiniteAxisValue { .. })
    ));
}

#[test]
fn control_frame_conversion_fails_on_infinite_axis_value() {
    let payload = wire::ControlPayload {
        axes: vec![wire::AxisSample {
            axis_id: 0,
            value: f32::INFINITY,
        }],
        edges: vec![],
    };
    let mut wire_frame: wire::ControlFrame = (&sample_frame(ControlPayload::default())).into();
    wire_frame.payload = Some(payload);
    let result = ScopedControlFrame::try_from(wire_frame);
    assert!(matches!(
        result,
        Err(ConvertError::NonFiniteAxisValue { .. })
    ));
}

#[test]
fn decode_control_frame_envelope_fails_on_unsupported_schema_version() {
    let frame = sample_frame(ControlPayload::default());
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION + 1,
        payload: Some(wire::envelope::Payload::ControlFrame((&frame).into())),
    };
    let bytes = envelope.encode_to_vec();
    let result = decode_control_frame_envelope(&bytes);
    assert!(matches!(
        result,
        Err(DecodeError::Convert(ConvertError::UnsupportedSchemaVersion {
            expected: SCHEMA_VERSION,
            found,
        })) if found == SCHEMA_VERSION + 1
    ));
}

#[test]
fn length_delimited_envelope_roundtrips_and_leaves_no_remainder() {
    let frame = sample_frame(ControlPayload::default());
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ControlFrame((&frame).into())),
    };
    let bytes = encode_envelope_length_delimited(&envelope);
    let (decoded, rest) = decode_envelope_length_delimited(&bytes).expect("valid frame");
    assert_eq!(decoded, envelope);
    assert!(rest.is_empty());
}

#[test]
fn length_delimited_envelope_leaves_subsequent_bytes_for_next_frame() {
    let frame = sample_frame(ControlPayload::default());
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ControlFrame((&frame).into())),
    };
    let mut bytes = encode_envelope_length_delimited(&envelope);
    let second = encode_envelope_length_delimited(&envelope);
    bytes.extend_from_slice(&second);

    let (first_decoded, rest) = decode_envelope_length_delimited(&bytes).expect("valid frame");
    assert_eq!(first_decoded, envelope);
    let (second_decoded, rest2) = decode_envelope_length_delimited(rest).expect("valid frame");
    assert_eq!(second_decoded, envelope);
    assert!(rest2.is_empty());
}

#[test]
fn decode_length_delimited_fails_on_garbage_bytes() {
    let result = decode_envelope_length_delimited(&[0xFF, 0xFF, 0xFF]);
    assert!(matches!(result, Err(DecodeError::Prost { .. })));
}

mod proptests {
    use crate::control::{
        ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame,
    };
    use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
    use crate::wire;
    use pilotage_timing::MonoTimestamp;
    use proptest::prelude::*;

    fn arb_axis_value() -> impl Strategy<Value = f32> {
        // Finite f32 in [-1.0, 1.0], matching the domain's canonical axis
        // convention; NaN/inf are excluded because they are not valid
        // control-frame axis samples.
        (-1.0f32..=1.0f32).prop_filter("must be finite", |v| v.is_finite())
    }

    fn arb_button_edge() -> impl Strategy<Value = ButtonEdge> {
        prop_oneof![Just(ButtonEdge::Pressed), Just(ButtonEdge::Released)]
    }

    fn arb_payload() -> impl Strategy<Value = ControlPayload> {
        (
            prop::collection::vec((any::<u16>(), arb_axis_value()), 0..8),
            prop::collection::vec((any::<u16>(), arb_button_edge()), 0..8),
        )
            .prop_map(|(axes, edges)| ControlPayload {
                axes: axes
                    .into_iter()
                    .map(|(id, value)| (LogicalAxisId::new(id), value))
                    .collect(),
                edges: edges
                    .into_iter()
                    .map(|(id, edge)| (LogicalButtonId::new(id), edge))
                    .collect(),
            })
    }

    fn arb_control_frame() -> impl Strategy<Value = ScopedControlFrame> {
        (
            any::<u64>(),
            any::<u64>(),
            any::<u64>(),
            any::<u32>(),
            any::<u64>(),
            any::<u32>(),
            arb_payload(),
        )
            .prop_map(
                |(
                    session,
                    vehicle,
                    generation,
                    sequence,
                    sampled_at,
                    profile_revision,
                    payload,
                )| {
                    ScopedControlFrame {
                        session: SessionId::new(session),
                        vehicle: VehicleId::new(vehicle),
                        scope: ScopeId::new("vehicle.motion"),
                        generation: Generation::new(generation),
                        sequence: SequenceNum::new(sequence),
                        sampled_at: MonoTimestamp::from_nanos(sampled_at),
                        profile_revision,
                        payload,
                        intent: None,
                        actions: vec![],
                    }
                },
            )
    }

    proptest! {
        #[test]
        fn control_frame_roundtrips_through_wire(frame in arb_control_frame()) {
            let wire_frame: wire::ControlFrame = (&frame).into();
            let back = ScopedControlFrame::try_from(wire_frame);
            prop_assert_eq!(back, Ok(frame));
        }
    }
}
