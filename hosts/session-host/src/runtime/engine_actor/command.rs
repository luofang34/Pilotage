//! The engine actor's inbound command vocabulary: everything a
//! connection task or an in-process principal asks the single
//! engine-owning task to do.

use pilotage_protocol::{VehicleId, wire};
use pilotage_session::{ClientKey, DomainEnvelope};
use pilotage_timing::MonoTimestamp;
use tokio::sync::{mpsc, oneshot};

use crate::runtime::connection::ToConnection;

/// One command a connection task submits to the engine actor.
#[derive(Debug)]
pub enum ToEngine {
    /// A client connected; the actor should register its outbound sender in
    /// the client registry keyed on `client`.
    ClientConnected {
        /// Driver-assigned key for the new connection.
        client: ClientKey,
        /// Outbound sender the actor unicasts/broadcasts through.
        sender: mpsc::Sender<ToConnection>,
    },
    /// A decoded client message ready for [`SessionEngine::handle_client_message`].
    ///
    /// [`SessionEngine::handle_client_message`]: pilotage_session::SessionEngine::handle_client_message
    ClientMessage {
        /// Sender of the message.
        client: ClientKey,
        /// The decoded message.
        message: DomainEnvelope,
        /// The driver's receive timestamp, for staleness/latency accounting.
        now: MonoTimestamp,
    },
    /// Navigation guidance for one vehicle, published by the host's
    /// navigation component rather than an FC adapter (ADR-0031). The
    /// actor holds the latest state and attaches it to that vehicle's
    /// outgoing telemetry until it is replaced or cleared.
    NavGuidance {
        /// The vehicle the guidance describes.
        vehicle: VehicleId,
        /// The stamped guidance state, or `None` to clear it: no plan
        /// being flown means the field goes absent on the wire, never
        /// zeroed or centered.
        state: Option<Box<wire::NavGuidanceState>>,
    },
    /// A one-shot latency summary request, used by the shutdown path to log
    /// the accumulated per-stage timings before the process exits.
    DumpLatencySummary {
        /// Where to send the formatted summary line.
        reply: oneshot::Sender<String>,
    },
}
