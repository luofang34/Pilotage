//! Host-to-client message payloads carried by unicast and broadcast actions.
//!
//! These are the domain-typed (not wire-typed) messages the engine decides to
//! send; the driver encodes each into a `pilotage.v1.Envelope` and writes it
//! on the appropriate WebTransport channel (ADR-0005). Keeping them domain
//! typed lets the engine remain sans-IO and lets the wire encoding live wholly
//! in `pilotage-protocol` and `pilotage-authority`.

use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{
    ControlActionResult, LeaseReleased, LeaseResponse, LinkLossCleared, Pong, ServerWelcome,
};

/// A message the engine directs at one client or at all clients.
///
/// The [`OutboundMessage::Authority`] arm carries the engine's own
/// [`AuthorityEffect`] rather than a pre-built wire event: the authority crate
/// owns the effect-to-`AuthorityEvent` conversion (its audit-trail
/// serialization is total over the effect enum), so the driver calls
/// `proto::AuthorityEvent::from(&effect)` at encode time. This keeps the
/// authority wire mapping in one place.
#[derive(Debug, Clone, PartialEq)]
pub enum OutboundMessage {
    /// The reply to a `ClientHello` (ADR-0005 handshake).
    Welcome(ServerWelcome),
    /// The reply to a `LeaseRequest` (ADR-0006).
    LeaseResponse(LeaseResponse),
    /// The acknowledgement of a `LeaseRelease` (ADR-0006): the sender may
    /// treat its authority as relinquished on receipt.
    LeaseReleased(LeaseReleased),
    /// The reliable notice that the host cleared its link-loss latch for one
    /// scope of a vehicle on a fresh generation (ADR-0012): the recovering
    /// client resumes live control only once it correlates this.
    LinkLossCleared(LinkLossCleared),
    /// The reply to a `Ping` (ADR-0009 RTT probe).
    Pong(Pong),
    /// The explicit outcome of one typed discrete action (CTRL-01), sent to
    /// the frame's sender on the reliable session stream after the adapter
    /// disposed of it.
    ControlActionResult(ControlActionResult),
    /// An authority event to be serialized and observed on the ordered
    /// authority stream (ADR-0006, ADR-0012). Carried as the source
    /// [`AuthorityEffect`] so the driver performs the canonical wire mapping.
    Authority(AuthorityEffect),
}
