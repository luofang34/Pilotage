//! Static configuration for a [`SessionEngine`] (ADR-0005, ADR-0009).
//!
//! Config is supplied once at construction and never mutated; it bundles the
//! handshake floor, the host version string echoed in capabilities, and the
//! per-call action cap that bounds the engine's output.
//!
//! [`SessionEngine`]: crate::SessionEngine

/// Immutable knobs the engine reads while deciding actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// Lowest `pilotage.v1` schema version the host will serve. A
    /// `ClientHello` advertising a lower `protocol_version` is closed with
    /// [`CloseReason::UnsupportedProtocolVersion`].
    ///
    /// [`CloseReason::UnsupportedProtocolVersion`]:
    /// crate::CloseReason::UnsupportedProtocolVersion
    pub required_protocol_version: u32,
    /// Human-readable host version echoed into `HostCapabilities.host_version`
    /// (ADR-0008), for compatibility negotiation and diagnostics only.
    pub host_version: String,
    /// Hard cap on the number of [`SessionAction`]s a single
    /// `handle_client_message` or `handle_tick` call may emit.
    ///
    /// The engine is driven by untrusted client input; without a cap a
    /// pathological message (or a burst of authority effects) could grow the
    /// returned vector unboundedly. When the cap is reached the engine stops
    /// appending and drops further actions for that call — a dropped action is
    /// a correctness signal the driver counts, mirroring the bounded-channel
    /// discipline of the async layer. The cap is generous relative to the
    /// worst realistic fan-out of one message (a handshake, a lease grant plus
    /// its broadcast, or a disconnect releasing every held scope), so hitting
    /// it means a scope count or client population far outside design
    /// envelope.
    ///
    /// [`SessionAction`]: crate::SessionAction
    pub max_actions_per_call: usize,
}

impl SessionConfig {
    /// The default per-call action cap.
    ///
    /// Chosen well above the fan-out of any single client message in the
    /// increment-0 loopback: even a disconnect releasing every scope of a
    /// many-vehicle adapter, each producing a link-state and a revoke
    /// broadcast, stays comfortably under this bound.
    pub const DEFAULT_MAX_ACTIONS_PER_CALL: usize = 256;

    /// Constructs a config with the given handshake floor and host version,
    /// using [`SessionConfig::DEFAULT_MAX_ACTIONS_PER_CALL`].
    #[must_use]
    pub fn new(required_protocol_version: u32, host_version: impl Into<String>) -> Self {
        Self {
            required_protocol_version,
            host_version: host_version.into(),
            max_actions_per_call: Self::DEFAULT_MAX_ACTIONS_PER_CALL,
        }
    }

    /// Overrides the per-call action cap.
    #[must_use]
    pub fn with_max_actions_per_call(mut self, cap: usize) -> Self {
        self.max_actions_per_call = cap;
        self
    }
}
