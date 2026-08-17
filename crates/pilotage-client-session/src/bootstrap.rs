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

/// The control-profile identity a client announces: the id, revisions,
/// and content digest of the mapping that produces its frames, so
/// control evidence binds every frame to exactly that mapping
/// (INPUT-01). The default names the fixed built-in stick mapping; a
/// shell running the shared control runtime replaces it with the
/// runtime's own compiled identity before the first grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileIdentity {
    /// Stable profile document id.
    pub profile_id: String,
    /// The profile document's revision.
    pub profile_revision: u32,
    /// The session's activation revision for this mapping.
    pub activation_revision: u32,
    /// Content digest of the mapping that produces the frames.
    pub digest: [u8; 32],
}

impl Default for ProfileIdentity {
    fn default() -> Self {
        let profile_id = "pilotage-native-sticks/v1".to_owned();
        Self {
            digest: pilotage_input::content_digest(profile_id.as_bytes()),
            profile_id,
            profile_revision: 1,
            activation_revision: 1,
        }
    }
}

/// The activation announcement that binds this connection's control to
/// its announced mapping. The host refuses actions and typed frames
/// from a connection that never announced one.
pub(crate) fn profile_activation(session_id: u64, profile: &ProfileIdentity) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ProfileActivation(
            wire::ProfileActivation {
                session: Some(wire::SessionId { value: session_id }),
                profile_id: profile.profile_id.clone(),
                profile_revision: profile.profile_revision,
                activation_revision: profile.activation_revision,
                digest: profile.digest.to_vec(),
                // The device mapping is part of the announced identity's
                // own digest today, like the browser's keyboard profile:
                // no separate device-profile document exists to name.
                device_profile_id: String::new(),
                device_profile_revision: 0,
                device_digest: Vec::new(),
            },
        )),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

/// A non-holder's ask for a held scope (CLIENT-09).
pub(crate) fn transfer_request(vehicle_id: u64, scope: &str) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ScopeTransferRequest(
            wire::ScopeTransferRequest {
                vehicle: Some(wire::VehicleId { value: vehicle_id }),
                scope: Some(wire::ScopeId {
                    value: scope.to_owned(),
                }),
            },
        )),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

/// The holder's offer of its held scope to another principal.
pub(crate) fn transfer_offer(vehicle_id: u64, scope: &str, to_principal: u64) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ScopeTransferOffer(
            wire::ScopeTransferOffer {
                vehicle: Some(wire::VehicleId { value: vehicle_id }),
                scope: Some(wire::ScopeId {
                    value: scope.to_owned(),
                }),
                to_principal: Some(wire::PrincipalId {
                    value: to_principal,
                }),
            },
        )),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

/// The offered principal's acceptance, committing the transfer.
pub(crate) fn transfer_accept(vehicle_id: u64, scope: &str) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ScopeTransferAccept(
            wire::ScopeTransferAccept {
                vehicle: Some(wire::VehicleId { value: vehicle_id }),
                scope: Some(wire::ScopeId {
                    value: scope.to_owned(),
                }),
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
