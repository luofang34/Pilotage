//! Unit tests for session-bootstrap domain<->wire conversions
//! (`session_convert.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::convert::{ConvertError, SCHEMA_VERSION};
use crate::ids::{Generation, PrincipalId, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::wire;
use pilotage_timing::MonoTimestamp;
use prost::Message;

#[test]
fn client_hello_roundtrips() {
    let hello = ClientHello {
        protocol_version: 1,
        client_name: "browser-client/0.1".to_owned(),
        join_token: vec![1, 2, 3, 4],
    };
    let wire_hello: wire::ClientHello = (&hello).into();
    let back = ClientHello::from(wire_hello);
    assert_eq!(back, hello);
}

#[test]
fn client_hello_roundtrips_with_empty_token() {
    let hello = ClientHello {
        protocol_version: 1,
        client_name: String::new(),
        join_token: Vec::new(),
    };
    let wire_hello: wire::ClientHello = (&hello).into();
    let back = ClientHello::from(wire_hello);
    assert_eq!(back, hello);
}

fn sample_snapshot(holder: Option<u64>) -> ScopeHolderSnapshot {
    ScopeHolderSnapshot {
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        holder: holder.map(PrincipalId::new),
        generation: Generation::new(2),
    }
}

#[test]
fn scope_holder_snapshot_roundtrips_with_holder() {
    let snapshot = sample_snapshot(Some(9));
    let wire_snapshot: wire::ScopeHolderSnapshot = (&snapshot).into();
    let back = ScopeHolderSnapshot::try_from(wire_snapshot).expect("valid conversion");
    assert_eq!(back, snapshot);
}

#[test]
fn scope_holder_snapshot_roundtrips_without_holder() {
    let snapshot = sample_snapshot(None);
    let wire_snapshot: wire::ScopeHolderSnapshot = (&snapshot).into();
    let back = ScopeHolderSnapshot::try_from(wire_snapshot).expect("valid conversion");
    assert_eq!(back, snapshot);
}

#[test]
fn scope_holder_snapshot_conversion_fails_on_missing_scope() {
    let mut wire_snapshot: wire::ScopeHolderSnapshot = (&sample_snapshot(None)).into();
    wire_snapshot.scope = None;
    let result = ScopeHolderSnapshot::try_from(wire_snapshot);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField { field: "scope", .. })
    ));
}

fn sample_welcome() -> ServerWelcome {
    ServerWelcome {
        session: SessionId::new(1),
        principal: PrincipalId::new(2),
        host_capabilities: wire::HostCapabilities {
            host_version: "0.1.0".to_owned(),
            vehicles: vec![],
            supported_modes: vec![],
        },
        scope_holders: vec![sample_snapshot(Some(9)), sample_snapshot(None)],
    }
}

#[test]
fn server_welcome_roundtrips() {
    let welcome = sample_welcome();
    let wire_welcome: wire::ServerWelcome = (&welcome).into();
    let back = ServerWelcome::try_from(wire_welcome).expect("valid conversion");
    assert_eq!(back, welcome);
}

#[test]
fn server_welcome_conversion_fails_on_missing_session() {
    let mut wire_welcome: wire::ServerWelcome = (&sample_welcome()).into();
    wire_welcome.session = None;
    let result = ServerWelcome::try_from(wire_welcome);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField {
            field: "session",
            ..
        })
    ));
}

#[test]
fn lease_request_roundtrips() {
    let request = LeaseRequest {
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
    };
    let wire_request: wire::LeaseRequest = (&request).into();
    let back = LeaseRequest::try_from(wire_request).expect("valid conversion");
    assert_eq!(back, request);
}

#[test]
fn lease_request_conversion_fails_on_missing_vehicle() {
    let mut wire_request: wire::LeaseRequest = (&LeaseRequest {
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
    })
        .into();
    wire_request.vehicle = None;
    let result = LeaseRequest::try_from(wire_request);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField {
            field: "vehicle",
            ..
        })
    ));
}

fn sample_lease_response(granted: bool, reason: Option<LeaseDenialReason>) -> LeaseResponse {
    LeaseResponse {
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        granted,
        generation: Generation::new(3),
        reason,
    }
}

#[test]
fn lease_response_roundtrips_on_grant() {
    let response = sample_lease_response(true, None);
    let wire_response: wire::LeaseResponse = (&response).into();
    let back = LeaseResponse::try_from(wire_response).expect("valid conversion");
    assert_eq!(back, response);
}

#[test]
fn lease_response_roundtrips_on_denial() {
    let response = sample_lease_response(false, Some(LeaseDenialReason::AlreadyHeld));
    let wire_response: wire::LeaseResponse = (&response).into();
    let back = LeaseResponse::try_from(wire_response).expect("valid conversion");
    assert_eq!(back, response);
}

#[test]
fn lease_response_conversion_fails_on_unspecified_denial_reason() {
    let mut wire_response: wire::LeaseResponse = (&sample_lease_response(false, None)).into();
    wire_response.reason = wire::LeaseDenialReason::Unspecified as i32;
    let result = LeaseResponse::try_from(wire_response);
    assert!(matches!(result, Err(ConvertError::UnknownEnum { .. })));
}

#[test]
fn lease_response_grant_ignores_reason_field() {
    // A grant is not required to carry a concrete denial reason; even an
    // Unspecified reason on the wire must not fail conversion when granted.
    let mut wire_response: wire::LeaseResponse = (&sample_lease_response(true, None)).into();
    wire_response.reason = wire::LeaseDenialReason::Unspecified as i32;
    let back = LeaseResponse::try_from(wire_response).expect("valid conversion");
    assert_eq!(back.reason, None);
}

#[test]
fn ping_roundtrips() {
    let ping = Ping {
        nonce: 42,
        sender_sent_at: MonoTimestamp::from_nanos(100),
    };
    let wire_ping: wire::Ping = (&ping).into();
    let back = Ping::try_from(wire_ping).expect("valid conversion");
    assert_eq!(back, ping);
}

#[test]
fn ping_conversion_fails_on_missing_timestamp() {
    let mut wire_ping: wire::Ping = (&Ping {
        nonce: 1,
        sender_sent_at: MonoTimestamp::from_nanos(1),
    })
        .into();
    wire_ping.sender_sent_at = None;
    let result = Ping::try_from(wire_ping);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField {
            field: "sender_sent_at",
            ..
        })
    ));
}

#[test]
fn pong_roundtrips() {
    let pong = Pong {
        nonce: 42,
        echoed_sender_sent_at: MonoTimestamp::from_nanos(100),
        responder_sent_at: MonoTimestamp::from_nanos(150),
    };
    let wire_pong: wire::Pong = (&pong).into();
    let back = Pong::try_from(wire_pong).expect("valid conversion");
    assert_eq!(back, pong);
}

#[test]
fn pong_conversion_fails_on_missing_responder_timestamp() {
    let mut wire_pong: wire::Pong = (&Pong {
        nonce: 1,
        echoed_sender_sent_at: MonoTimestamp::from_nanos(1),
        responder_sent_at: MonoTimestamp::from_nanos(2),
    })
        .into();
    wire_pong.responder_sent_at = None;
    let result = Pong::try_from(wire_pong);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField {
            field: "responder_sent_at",
            ..
        })
    ));
}

fn sample_frame_rejected() -> FrameRejected {
    FrameRejected {
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        sequence: SequenceNum::new(7),
        reason: FrameRejectionReason::StaleGeneration,
        current_generation: Generation::new(5),
    }
}

#[test]
fn frame_rejected_roundtrips() {
    let rejected = sample_frame_rejected();
    let wire_rejected: wire::FrameRejected = (&rejected).into();
    let back = FrameRejected::try_from(wire_rejected).expect("valid conversion");
    assert_eq!(back, rejected);
}

#[test]
fn frame_rejected_conversion_fails_on_unspecified_reason() {
    let mut wire_rejected: wire::FrameRejected = (&sample_frame_rejected()).into();
    wire_rejected.reason = wire::FrameRejectionReason::Unspecified as i32;
    let result = FrameRejected::try_from(wire_rejected);
    assert!(matches!(result, Err(ConvertError::UnknownEnum { .. })));
}

#[test]
fn frame_rejected_conversion_fails_on_missing_scope() {
    let mut wire_rejected: wire::FrameRejected = (&sample_frame_rejected()).into();
    wire_rejected.scope = None;
    let result = FrameRejected::try_from(wire_rejected);
    assert!(matches!(
        result,
        Err(ConvertError::MissingField { field: "scope", .. })
    ));
}

#[test]
fn envelope_roundtrips_for_client_hello_arm() {
    let hello = wire::ClientHello {
        protocol_version: 1,
        client_name: "browser-client/0.1".to_owned(),
        join_token: vec![9, 9],
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ClientHello(hello.clone())),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::ClientHello(hello))
    );
}

#[test]
fn envelope_roundtrips_for_lease_request_arm() {
    let request = wire::LeaseRequest {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: "vehicle.motion".to_owned(),
        }),
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::LeaseRequest(request.clone())),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::LeaseRequest(request))
    );
}

#[test]
fn envelope_roundtrips_for_ping_and_pong_arms() {
    let ping = wire::Ping {
        nonce: 1,
        sender_sent_at: Some(wire::MonoTimestamp { nanos: 10 }),
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::Ping(ping)),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(decoded.payload, Some(wire::envelope::Payload::Ping(ping)));

    let pong = wire::Pong {
        nonce: 1,
        echoed_sender_sent_at: Some(wire::MonoTimestamp { nanos: 10 }),
        responder_sent_at: Some(wire::MonoTimestamp { nanos: 20 }),
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::Pong(pong)),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(decoded.payload, Some(wire::envelope::Payload::Pong(pong)));
}

#[test]
fn envelope_roundtrips_for_frame_rejected_arm() {
    let rejected = wire::FrameRejected {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: "vehicle.motion".to_owned(),
        }),
        sequence: Some(wire::SequenceNum { value: 7 }),
        reason: wire::FrameRejectionReason::TooOld as i32,
        current_generation: Some(wire::Generation { value: 5 }),
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::FrameRejected(rejected.clone())),
    };
    let bytes = envelope.encode_to_vec();
    let decoded = wire::Envelope::decode(bytes.as_slice()).expect("valid envelope");
    assert_eq!(
        decoded.payload,
        Some(wire::envelope::Payload::FrameRejected(rejected))
    );
}

mod proptests {
    use crate::ids::{Generation, ScopeId, SequenceNum, VehicleId};
    use crate::session::{FrameRejected, FrameRejectionReason};
    use crate::wire;
    use proptest::prelude::*;

    fn arb_reason() -> impl Strategy<Value = FrameRejectionReason> {
        prop_oneof![
            Just(FrameRejectionReason::StaleGeneration),
            Just(FrameRejectionReason::NoHolder),
            Just(FrameRejectionReason::UnknownScope),
            Just(FrameRejectionReason::TooOld),
        ]
    }

    fn arb_frame_rejected() -> impl Strategy<Value = FrameRejected> {
        (any::<u64>(), any::<u64>(), any::<u32>(), arb_reason()).prop_map(
            |(vehicle, generation, sequence, reason)| FrameRejected {
                vehicle: VehicleId::new(vehicle),
                scope: ScopeId::new("vehicle.motion"),
                sequence: SequenceNum::new(sequence),
                reason,
                current_generation: Generation::new(generation),
            },
        )
    }

    proptest! {
        #[test]
        fn frame_rejected_roundtrips_through_wire(rejected in arb_frame_rejected()) {
            let wire_rejected: wire::FrameRejected = (&rejected).into();
            let back = FrameRejected::try_from(wire_rejected);
            prop_assert_eq!(back, Ok(rejected));
        }
    }
}
