//! Mission intents (ADR-0052): a client sends an intent to a host's mission
//! lane on the reliable session stream; the lane's receipt and status come
//! back as module events.

use pilotage_protocol::{SCHEMA_VERSION, wire};

use crate::{ClientAction, ClientEngine, ClientPhase};

impl ClientEngine {
    /// The bytes that send `intent` to the host's mission lane, or nothing
    /// before the host admitted this session.
    #[must_use]
    pub fn mission_intent(&self, intent: wire::MissionIntent) -> Vec<ClientAction> {
        if *self.phase() != ClientPhase::Admitted {
            return Vec::new();
        }
        let envelope = wire::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::MissionIntent(intent)),
        };
        vec![ClientAction::SendBootstrap(
            pilotage_protocol::encode_envelope_length_delimited(&envelope),
        )]
    }
}
