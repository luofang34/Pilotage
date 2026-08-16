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

/// The native client's control-profile identity: the built-in game
/// controller mapping, versioned. The digest is the content digest of
/// this identity, so control evidence binds every frame to exactly the
/// mapping that produced it (INPUT-01).
pub(crate) const NATIVE_PROFILE_ID: &str = "pilotage-native-sticks/v1";

/// The revisions announced for the built-in mapping. Both advance only
/// when the mapping itself changes.
pub(crate) const NATIVE_PROFILE_REVISION: u32 = 1;
pub(crate) const NATIVE_ACTIVATION_REVISION: u32 = 1;

/// The activation announcement that binds this connection's control to
/// the built-in mapping. The host refuses actions and typed frames from
/// a connection that never announced one.
pub(crate) fn profile_activation(session_id: u64) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ProfileActivation(
            wire::ProfileActivation {
                session: Some(wire::SessionId { value: session_id }),
                profile_id: NATIVE_PROFILE_ID.to_owned(),
                profile_revision: NATIVE_PROFILE_REVISION,
                activation_revision: NATIVE_ACTIVATION_REVISION,
                digest: pilotage_input::content_digest(NATIVE_PROFILE_ID.as_bytes()).to_vec(),
                // The game-controller mapping is a fixed built-in of the
                // client binary, like the browser's keyboard profile: no
                // separate device-profile document exists to name.
                device_profile_id: String::new(),
                device_profile_revision: 0,
                device_digest: Vec::new(),
            },
        )),
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
