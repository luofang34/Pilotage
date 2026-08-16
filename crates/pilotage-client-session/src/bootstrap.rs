//! Bootstrap envelope construction: hello, lease request, lease release,
//! ping.

use pilotage_protocol::wire;

use crate::control::SCHEMA_VERSION;

/// The hello this client opens every session with.
pub(crate) fn hello(client_name: &str) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ClientHello(wire::ClientHello {
            protocol_version: pilotage_protocol::SESSION_PROTOCOL_VERSION,
            client_name: client_name.to_owned(),
            join_token: Vec::new(),
        })),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

/// A lease request for one (vehicle, scope).
pub(crate) fn lease_request(vehicle_id: u64, scope: &str) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::LeaseRequest(wire::LeaseRequest {
            vehicle: Some(wire::VehicleId { value: vehicle_id }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
        })),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

/// A voluntary release of one held (vehicle, scope).
pub(crate) fn lease_release(vehicle_id: u64, scope: &str) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::LeaseRelease(wire::LeaseRelease {
            vehicle: Some(wire::VehicleId { value: vehicle_id }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
        })),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}
