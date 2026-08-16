//! The control lane: fencing state and frame construction.
//!
//! A control frame is only meaningful inside a granted lease: it carries
//! the session, vehicle, scope, and the grant's fencing generation, and the
//! host rejects anything stale. The lane owns that stamping so a shell can
//! never send a frame the engine did not fence.

use pilotage_protocol::wire;
use prost::Message;

/// Wire schema version stamped on every envelope this crate builds.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// One typed command for the lane to send. Exactly one representation per
/// frame: the typed intent, a discrete action, or the legacy axis payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    /// A typed control intent (velocity, attitude-thrust, ...).
    Intent(wire::ControlIntent),
    /// The legacy numeric axis/button payload.
    Legacy(wire::ControlPayload),
}

/// Fencing state for one granted (vehicle, scope).
#[derive(Debug, Clone, PartialEq)]
pub struct ControlLane {
    session_id: u64,
    vehicle_id: u64,
    scope: String,
    generation: u64,
    sequence: u32,
    profile_revision: u32,
    activation_revision: u32,
    next_action_id: u32,
}

impl ControlLane {
    /// Opens a lane from a grant. `generation` is the grant's fencing
    /// generation; frames stamped with anything else are rejected by the
    /// host, which is the point.
    #[must_use]
    pub fn new(session_id: u64, vehicle_id: u64, scope: String, generation: u64) -> Self {
        Self {
            session_id,
            vehicle_id,
            scope,
            generation,
            sequence: 0,
            profile_revision: 0,
            activation_revision: 0,
            next_action_id: 0,
        }
    }

    /// Binds the lane to an activated control profile. Every later frame
    /// carries these revisions, tying control evidence to the exact
    /// mapping that produced it.
    pub fn bind_profile(&mut self, profile_revision: u32, activation_revision: u32) {
        self.profile_revision = profile_revision;
        self.activation_revision = activation_revision;
    }

    /// The scope this lane is fenced to.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The vehicle this lane is fenced to.
    #[must_use]
    pub fn vehicle_id(&self) -> u64 {
        self.vehicle_id
    }

    /// Builds the next control-frame datagram. The sequence advances by
    /// one per frame and wraps; the host's staleness window, not the
    /// counter range, is the real bound.
    #[must_use]
    pub fn frame(&mut self, command: ControlCommand, sampled_at_nanos: u64) -> Vec<u8> {
        self.sequence = self.sequence.wrapping_add(1);
        let (payload, intent) = match command {
            ControlCommand::Intent(intent) => (None, Some(intent)),
            ControlCommand::Legacy(payload) => (Some(payload), None),
        };
        let envelope = wire::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::ControlFrame(wire::ControlFrame {
                session: Some(wire::SessionId {
                    value: self.session_id,
                }),
                vehicle: Some(wire::VehicleId {
                    value: self.vehicle_id,
                }),
                scope: Some(wire::ScopeId {
                    value: self.scope.clone(),
                }),
                generation: Some(wire::Generation {
                    value: self.generation,
                }),
                sequence: Some(wire::SequenceNum {
                    value: self.sequence,
                }),
                sampled_at: Some(wire::MonoTimestamp {
                    nanos: sampled_at_nanos,
                }),
                profile_revision: self.profile_revision,
                activation_revision: self.activation_revision,
                payload,
                intent,
                actions: Vec::new(),
            })),
        };
        envelope.encode_to_vec()
    }

    /// Builds a reliable-stream discrete action command. The action id is
    /// nonzero and advances per command, so every result correlates.
    #[must_use]
    pub fn action_command(&mut self, request: wire::ControlActionRequest) -> Vec<u8> {
        self.next_action_id = self.next_action_id.wrapping_add(1);
        let mut request = request;
        if request.action_id == 0 {
            request.action_id = self.next_action_id;
        }
        let envelope = wire::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::ControlActionCommand(
                wire::ControlActionCommand {
                    session: Some(wire::SessionId {
                        value: self.session_id,
                    }),
                    vehicle: Some(wire::VehicleId {
                        value: self.vehicle_id,
                    }),
                    scope: Some(wire::ScopeId {
                        value: self.scope.clone(),
                    }),
                    generation: Some(wire::Generation {
                        value: self.generation,
                    }),
                    activation_revision: self.activation_revision,
                    request: Some(request),
                },
            )),
        };
        pilotage_protocol::encode_envelope_length_delimited(&envelope)
    }

    /// Restores fencing counters exactly, for tests that must reproduce a
    /// recorded frame byte for byte.
    #[cfg(test)]
    pub(crate) fn restore_counters(&mut self, sequence: u32, next_action_id: u32) {
        self.sequence = sequence;
        self.next_action_id = next_action_id;
    }
}
