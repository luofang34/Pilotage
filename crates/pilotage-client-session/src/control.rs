//! The control lane: fencing state and frame construction.
//!
//! A control frame is only meaningful inside a granted lease: it carries
//! the session, vehicle, scope, and the grant's fencing generation, and the
//! host rejects anything stale. The lane owns that stamping so a shell can
//! never send a frame the engine did not fence.

use pilotage_protocol::wire;
use prost::Message;

use crate::ActionRequestIdentity;

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
    last_sampled_at_nanos: u64,
    control_window_id: u64,
    require_control_window: bool,
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
            last_sampled_at_nanos: 0,
            control_window_id: 0,
            require_control_window: false,
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

    /// The grant's fencing generation this lane stamps on its frames.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The vehicle this lane is fenced to.
    #[must_use]
    pub fn vehicle_id(&self) -> u64 {
        self.vehicle_id
    }

    pub(crate) fn set_control_window(&mut self, id: u64, required: bool) {
        self.control_window_id = id;
        self.require_control_window = required;
    }

    fn command_timing(&self) -> wire::CommandTiming {
        let timing = pilotage_protocol::CommandTiming {
            control_window_id: self.control_window_id,
            require_control_window: self.require_control_window,
            ..pilotage_protocol::CommandTiming::default()
        };
        timing.into()
    }

    fn action_timing(&self, action: i32) -> wire::CommandTiming {
        let mut timing = self.command_timing();
        if matches!(
            wire::ControlAction::try_from(action),
            Ok(wire::ControlAction::Takeoff | wire::ControlAction::Land)
        ) {
            // Admission stays short; a finite procedure has its own completion budget.
            timing.active_for_ms = 0;
            timing.completion_timeout_ms = 30_000;
        }
        timing
    }

    /// Builds the next control-frame datagram. The sequence advances by
    /// one per frame and wraps; the host's staleness window, not the
    /// counter range, is the real bound.
    #[must_use]
    pub fn frame(&mut self, command: ControlCommand, sampled_at_nanos: u64) -> Vec<u8> {
        self.last_sampled_at_nanos = sampled_at_nanos;
        self.sequence = self.sequence.wrapping_add(1);
        let (payload, intent) = match command {
            ControlCommand::Intent(intent) => (None, Some(intent)),
            ControlCommand::Legacy(payload) => (Some(payload), None),
        };
        let envelope = wire::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::ControlFrame(wire::ControlFrame {
                timing: self.require_control_window.then(|| self.command_timing()),
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
        self.action_command_at(request, self.last_sampled_at_nanos)
    }

    /// Build an action from a newly sampled input; transport retries retain its bytes.
    #[must_use]
    pub fn action_command_at(
        &mut self,
        request: wire::ControlActionRequest,
        sampled_at_nanos: u64,
    ) -> Vec<u8> {
        self.prepare_action_at(request, sampled_at_nanos).1
    }

    pub(crate) fn prepare_action_at(
        &mut self,
        request: wire::ControlActionRequest,
        sampled_at_nanos: u64,
    ) -> (ActionRequestIdentity, Vec<u8>) {
        self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
        let mut request = request;
        if request.action_id == 0 {
            request.action_id = self.next_action_id;
        }
        let identity = ActionRequestIdentity {
            session_id: self.session_id,
            vehicle_id: self.vehicle_id,
            scope: self.scope.clone(),
            generation: self.generation,
            sequence: 0,
            action_id: request.action_id,
        };
        let envelope = wire::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::ControlActionCommand(
                wire::ControlActionCommand {
                    sampled_at: Some(wire::MonoTimestamp {
                        nanos: sampled_at_nanos,
                    }),
                    timing: Some(self.action_timing(request.action)),
                    session: Some(wire::SessionId {
                        value: identity.session_id,
                    }),
                    vehicle: Some(wire::VehicleId {
                        value: identity.vehicle_id,
                    }),
                    scope: Some(wire::ScopeId {
                        value: identity.scope.clone(),
                    }),
                    generation: Some(wire::Generation {
                        value: identity.generation,
                    }),
                    activation_revision: self.activation_revision,
                    request: Some(request),
                },
            )),
        };
        (
            identity,
            pilotage_protocol::encode_envelope_length_delimited(&envelope),
        )
    }

    /// Restores fencing counters exactly, for tests that must reproduce a
    /// recorded frame byte for byte.
    #[cfg(test)]
    pub(crate) fn restore_counters(&mut self, sequence: u32, next_action_id: u32) {
        self.sequence = sequence;
        self.next_action_id = next_action_id;
    }
}
