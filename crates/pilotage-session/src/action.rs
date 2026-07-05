//! Actions the engine emits for the driver to execute (ADR-0005, ADR-0008).
//!
//! The [`SessionEngine`] never performs I/O; it returns [`SessionAction`]s and
//! the driver enacts them against the WebTransport session and the adapter.
//! Actions carry an already-built domain message or adapter frame so the
//! driver only encodes and writes — all decision logic stays in the engine.
//!
//! [`SessionEngine`]: crate::SessionEngine

use pilotage_protocol::{FrameRejected, ScopedControlFrame};

use crate::message::ClientKey;
use crate::outbound::OutboundMessage;

/// The result of one engine call: the actions to enact plus the count of
/// actions the per-call cap forced the engine to drop.
///
/// A non-zero `dropped` is a correctness signal, not noise: it means the
/// engine could not emit every action a message warranted (for example a
/// disconnect releasing more scopes than [`SessionConfig::max_actions_per_call`]
/// allows advanced authority state while its `HolderLinkLost` broadcasts were
/// truncated, so clients would not learn every released scope). The driver
/// MUST observe and count it, mirroring the bounded-channel drop discipline of
/// the async layer (ADR-0009: every queue drop is counted, never silent).
///
/// [`SessionConfig::max_actions_per_call`]: crate::SessionConfig::max_actions_per_call
#[derive(Debug, Clone, PartialEq)]
pub struct SessionOutcome {
    /// Actions the driver enacts, in emission order.
    pub actions: Vec<SessionAction>,
    /// Actions dropped because the per-call cap was reached. Non-zero means
    /// the returned `actions` are incomplete for this call.
    pub dropped: usize,
}

/// One unit of work the driver performs on the engine's behalf.
///
/// The enum's declaration order carries no meaning. The engine's ordering
/// guarantee is emission order only: within a single call, [`SessionOutcome::actions`]
/// lists actions in the order the driver must enact them (ADR-0012 events are
/// persisted in emission order). For example [`SessionEngine::on_lease`] pushes
/// a [`SessionAction::Broadcast`] before the [`SessionAction::SendToClient`]
/// lease response, even though `SendToClient` is declared first below.
///
/// [`SessionEngine::on_lease`]: crate::SessionEngine
#[derive(Debug, Clone, PartialEq)]
pub enum SessionAction {
    /// Send a message to exactly one client (unicast).
    SendToClient {
        /// Recipient connection.
        client: ClientKey,
        /// Message to encode and write to that client.
        envelope: OutboundMessage,
    },
    /// Apply a fence-verified, fresh control frame to the adapter.
    ApplyToAdapter {
        /// The frame the adapter should apply this tick.
        frame: ScopedControlFrame,
    },
    /// Send a message to every connected client (fan-out).
    ///
    /// Used for authority events, which every participant observes as the
    /// canonical ordered authority stream (ADR-0005 `authority-events`,
    /// ADR-0006).
    Broadcast {
        /// Message to encode and write to all clients.
        envelope: OutboundMessage,
    },
    /// Reject a control frame back to its sender only (never broadcast,
    /// ADR-0009).
    RejectFrame {
        /// The frame's sender.
        client: ClientKey,
        /// The typed rejection notice.
        rejection: FrameRejected,
    },
    /// Close a client's connection with a machine-readable reason.
    CloseClient {
        /// Connection to close.
        client: ClientKey,
        /// Why the connection is being closed.
        reason: CloseReason,
    },
}

/// Why the engine asked the driver to close a client connection.
///
/// Distinct from an authority [`RejectReason`] or [`FrameRejectionReason`]:
/// those keep the connection open. A `CloseReason` terminates the transport.
///
/// [`RejectReason`]: pilotage_authority::RejectReason
/// [`FrameRejectionReason`]: pilotage_protocol::FrameRejectionReason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The client sent a `ClientHello` advertising a protocol version the host
    /// cannot serve.
    UnsupportedProtocolVersion {
        /// Version the client advertised.
        offered: u32,
        /// Version the host requires.
        required: u32,
    },
    /// The client sent a second `ClientHello` on an already-welcomed
    /// connection, which the handshake does not permit.
    DuplicateHello,
    /// The client sent a domain message before completing the handshake.
    HandshakeNotComplete,
}
