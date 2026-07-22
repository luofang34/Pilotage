//! Driver-facing decoded-message vocabulary (ADR-0005, ADR-0014).
//!
//! The host binary owns the WebTransport I/O and the `prost` decode; it hands
//! the [`SessionEngine`] already-decoded domain messages so the engine stays
//! sans-IO (ADR-0002). [`DomainEnvelope`] is the closed set of client-origin
//! events the engine reacts to, including the synthetic
//! [`DomainEnvelope::Disconnect`] the driver raises when a client's transport
//! link drops.
//!
//! [`SessionEngine`]: crate::SessionEngine

use pilotage_protocol::{ClientHello, LeaseRelease, LeaseRequest, Ping, ScopedControlFrame};

/// Identifies one connected client (one WebTransport session) within a
/// [`SessionEngine`].
///
/// Assigned by the driver, opaque to the engine, and stable for the lifetime
/// of the transport connection. The engine keys all per-client state on it and
/// echoes it back in every [`SessionAction`] so the driver knows which
/// connection to write to.
///
/// [`SessionEngine`]: crate::SessionEngine
/// [`SessionAction`]: crate::SessionAction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientKey(u64);

impl ClientKey {
    /// Constructs a client key from a raw driver-assigned value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the key as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// A decoded client-origin message the driver submits to the engine.
///
/// This mirrors the client-to-host arms of the wire `Envelope` oneof
/// (ADR-0014) plus a synthetic [`DomainEnvelope::Disconnect`]. Host-to-client
/// messages (`ServerWelcome`, `LeaseResponse`, `Pong`, `FrameRejected`,
/// authority events) are never inputs; they appear only as
/// [`SessionAction`]s.
///
/// [`SessionAction`]: crate::SessionAction
#[derive(Debug, Clone, PartialEq)]
pub enum DomainEnvelope {
    /// The client's opening handshake, answered with a `ServerWelcome`.
    Hello(ClientHello),
    /// A request to lease a control scope, routed through the authority
    /// engine's grant path.
    Lease(LeaseRequest),
    /// A holder voluntarily relinquishing a scope, routed through the
    /// authority engine's release path (generation advances, link-loss
    /// policy engages) and acknowledged with a `LeaseReleased`.
    Release(LeaseRelease),
    /// A real-time control frame to be staleness-checked, fence-verified, and
    /// forwarded to the adapter or rejected.
    Frame(ScopedControlFrame),
    /// An RTT/offset probe, answered with a `Pong` stamped by the driver's
    /// clock.
    Ping(Ping),
    /// The client's announcement of a newly activated control profile
    /// (INPUT-01): the engine records it against the session so frames'
    /// `activation_revision` values are traceable to the exact profile
    /// identity, document revision, and content digest.
    ProfileActivation(pilotage_protocol::ProfileActivation),
    /// A typed discrete action on the RELIABLE ordered session stream
    /// (CTRL-01): validated against the sender's session, scope hold,
    /// fencing generation, and announced activation revision, answered with
    /// a `ControlActionResult`, and only then delivered to the adapter.
    ActionCommand(pilotage_protocol::ControlActionCommand),
    /// The driver observed the client's transport link drop; the engine
    /// releases every scope the client held via the authority engine's
    /// link-loss path.
    Disconnect,
}
