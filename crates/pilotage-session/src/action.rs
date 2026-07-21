//! Actions the engine emits for the driver to execute (ADR-0005, ADR-0008).
//!
//! The [`SessionEngine`] never performs I/O; it returns [`SessionAction`]s and
//! the driver enacts them against the WebTransport session and the adapter.
//! Actions carry an already-built domain message or adapter frame so the
//! driver only encodes and writes — all decision logic stays in the engine.
//!
//! [`SessionEngine`]: crate::SessionEngine

use pilotage_adapter_api::LinkLossPolicy;
use pilotage_protocol::{FrameRejected, Generation, ScopeId, ScopedControlFrame, VehicleId};

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
    /// Engage a vehicle's link-loss policy on the adapter because its holder was
    /// lost (ADR-0008, ADR-0010). Emitted exactly once per loss, after the
    /// fencing generation has already advanced, so the adapter drives its
    /// declared policy state (stop / zero velocity / hover-brake) a single
    /// time. The host does NOT re-transmit: a real flight controller ages
    /// that final setpoint and enters its own offboard-loss terminal state
    /// if the link stays absent. This action is emitted on the uncapped
    /// safety lane — it is never dropped behind the per-call action cap.
    EngageLinkLoss {
        /// Vehicle whose control link was lost.
        vehicle: VehicleId,
        /// The scope whose holder was lost.
        scope: ScopeId,
        /// The fencing generation in force after the loss advanced it;
        /// straggler frames from the lost holder are behind this value.
        generation: Generation,
        /// What judged the link lost.
        trigger: LinkLossTrigger,
        /// Policy the adapter should enact, selected and validated from the
        /// vehicle's declared `link_loss_actions` profile at engine
        /// construction ([`LinkLossPolicy::Neutralize`] when the profile
        /// declares it or declares nothing — the fail-closed floor).
        policy: LinkLossPolicy,
    },
    /// Clear a vehicle's engaged link-loss policy on the adapter — the only path
    /// back to normal control (ADR-0008). Emitted only after BOTH recovery
    /// conditions hold: a fresh lease generation installed a holder (grant,
    /// handover, or override — never reconnection alone), and that holder
    /// demonstrated the scope's activation condition with an accepted
    /// neutral-axes frame, so a client reconnecting with deflected sticks
    /// cannot revive motion.
    ClearLinkLoss {
        /// Vehicle whose scope is returning to normal control.
        vehicle: VehicleId,
        /// The specific scope recovering — link-loss is per-scope, so clearing
        /// one scope never returns another to control.
        scope: ScopeId,
        /// The fresh generation whose accepted neutral frame cleared the latch.
        /// The driver echoes it in the client-facing `LinkLossCleared` notice
        /// it broadcasts ONLY after the adapter confirms the clear.
        generation: Generation,
    },
}

/// What judged a holder's control link lost (ADR-0010): typed provenance for
/// the [`SessionAction::EngageLinkLoss`] it produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkLossTrigger {
    /// The holder's transport disconnected.
    HolderDisconnect,
    /// The holder stayed transport-connected but sent no accepted
    /// axis-bearing frame within the holder-silence window.
    HolderSilence,
    /// The holder's lease was revoked by an authority transition (an
    /// explicit release or an override displacing it).
    AuthorityRevoked,
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
