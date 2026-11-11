//! What an in-core client asks the engine to do: the session host's
//! engine-actor command vocabulary, for the commands the core's clients
//! send.

use pilotage_protocol::{VehicleId, wire};
use pilotage_session::{ClientKey, DomainEnvelope};
use pilotage_timing::MonoTimestamp;

/// One command an in-core client submits to the engine.
#[derive(Debug)]
pub(crate) enum ToEngine {
    /// A client connected; its messages queue under `client`.
    ClientConnected {
        /// The new client's key.
        client: ClientKey,
    },
    /// A client message for [`pilotage_session::SessionEngine::handle_client_message`].
    ClientMessage {
        /// Sender of the message.
        client: ClientKey,
        /// The message.
        message: Box<DomainEnvelope>,
        /// The time the client sent it, for staleness and latency.
        now: MonoTimestamp,
    },
    /// Navigation guidance for one vehicle (ADR-0031): the engine attaches
    /// the latest state to that vehicle's telemetry until it is replaced
    /// or cleared.
    NavGuidance {
        /// The vehicle the guidance describes.
        vehicle: VehicleId,
        /// The stamped guidance state, or `None` to clear it.
        state: Option<Box<wire::NavGuidanceState>>,
    },
}
