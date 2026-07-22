//! Static configuration for a [`SessionEngine`] (ADR-0005, ADR-0009).
//!
//! Config is supplied once at construction and never mutated; it bundles the
//! handshake floor, the host version string echoed in capabilities, the
//! per-call action cap that bounds the engine's output, and the holder-silence
//! watchdog window.
//!
//! [`SessionEngine`]: crate::SessionEngine

use std::time::Duration;

use pilotage_adapter_api::LinkLossPolicy;
use pilotage_protocol::VehicleId;

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
    /// Whether legacy numeric payload frames are admitted (the SIMULATION
    /// compatibility mode). OFF by default: production control is
    /// typed-only — legacy payloads bypass profile-activation binding and
    /// translate button edges into uncorrelated actions, so they exist
    /// only where a simulation harness explicitly opts in.
    pub legacy_compatibility: bool,
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
    /// How long a lease holder may go without a fresh, accepted control frame
    /// before the engine judges its control link lost and releases the scope
    /// (advancing the generation) — even though the WebTransport connection is
    /// still open.
    ///
    /// The QUIC keepalive holds an idle-but-open connection up for seconds, so a
    /// holder whose client froze (a backgrounded browser tab) would otherwise
    /// keep the lease and leave the vehicle on its last command indefinitely.
    /// This is that watchdog's window; RF/scheduling jitter tolerance belongs
    /// here (a value comfortably above the control cadence and the single-frame
    /// staleness bound), because once the deadline passes the holder is declared
    /// lost and the vehicle is neutralized — a decision that must not be undone
    /// by a late straggler frame.
    pub holder_silence: Duration,
    /// The recovery activation deadband, in thousandths of full axis
    /// deflection (stored as an integer so the config stays `Eq`).
    ///
    /// After a link-loss policy engages, a newly installed holder returns
    /// the vehicle to normal control only by demonstrating neutral input:
    /// an accepted frame whose every reported axis magnitude is at or
    /// below this deadband, with no pressed edges. Without the gate a
    /// client reconnecting with deflected sticks would drive the vehicle
    /// straight out of its neutralized state on the grant alone.
    pub activation_deadband_milli: u32,
    /// Explicitly configured link-loss policy per vehicle.
    ///
    /// A vehicle's `link_loss_actions` capability declares what the
    /// adapter *supports* — it is a menu, not a selection, and its order
    /// carries no meaning. The policy to *enact* is configured here and
    /// validated against that menu at engine construction; a configured
    /// policy the adapter never declared falls closed to
    /// `LinkLossPolicy::Neutralize` (the only universally safe floor), as
    /// does an unconfigured vehicle.
    pub link_loss_overrides: Vec<(VehicleId, LinkLossPolicy)>,
}

impl SessionConfig {
    /// The default per-call action cap.
    ///
    /// Chosen well above the fan-out of any single client message in the
    /// increment-0 loopback: even a disconnect releasing every scope of a
    /// many-vehicle adapter, each producing a link-state and a revoke
    /// broadcast, stays comfortably under this bound.
    pub const DEFAULT_MAX_ACTIONS_PER_CALL: usize = 256;

    /// The default holder-silence watchdog window.
    ///
    /// One second is many control frames at the ADR-0009 control cadence and
    /// well above the 250 ms single-frame staleness bound, so ordinary jitter or
    /// a few dropped frames never trip it, while a frozen or vanished client is
    /// caught within a second — long before the multi-second QUIC keepalive
    /// would (never, for a still-open connection) surface the loss.
    pub const DEFAULT_HOLDER_SILENCE: Duration = Duration::from_millis(1000);

    /// The default recovery activation deadband: 5% of full deflection,
    /// comfortably above stick-center noise and far below any deliberate
    /// command.
    pub const DEFAULT_ACTIVATION_DEADBAND_MILLI: u32 = 50;

    /// Constructs a config with the given handshake floor and host version,
    /// using [`SessionConfig::DEFAULT_MAX_ACTIONS_PER_CALL`] and
    /// [`SessionConfig::DEFAULT_HOLDER_SILENCE`].
    #[must_use]
    pub fn new(required_protocol_version: u32, host_version: impl Into<String>) -> Self {
        Self {
            legacy_compatibility: false,
            required_protocol_version,
            host_version: host_version.into(),
            max_actions_per_call: Self::DEFAULT_MAX_ACTIONS_PER_CALL,
            holder_silence: Self::DEFAULT_HOLDER_SILENCE,
            activation_deadband_milli: Self::DEFAULT_ACTIVATION_DEADBAND_MILLI,
            link_loss_overrides: Vec::new(),
        }
    }

    /// Overrides the per-call action cap.
    #[must_use]
    pub fn with_max_actions_per_call(mut self, cap: usize) -> Self {
        self.max_actions_per_call = cap;
        self
    }

    /// Enables the SIMULATION legacy-compatibility mode: legacy numeric
    /// payload frames are admitted and translated at the single
    /// compatibility boundary. Never the production default.
    #[must_use]
    pub fn with_legacy_compatibility(mut self, enabled: bool) -> Self {
        self.legacy_compatibility = enabled;
        self
    }

    /// Overrides the holder-silence watchdog window.
    #[must_use]
    pub fn with_holder_silence(mut self, holder_silence: Duration) -> Self {
        self.holder_silence = holder_silence;
        self
    }

    /// Overrides the recovery activation deadband (thousandths of full
    /// deflection).
    #[must_use]
    pub fn with_activation_deadband_milli(mut self, deadband_milli: u32) -> Self {
        self.activation_deadband_milli = deadband_milli;
        self
    }

    /// Configures the link-loss policy to enact for `vehicle`. Validated
    /// against the vehicle's declared `link_loss_actions` at engine
    /// construction; an undeclared configuration falls closed to
    /// `Neutralize`.
    #[must_use]
    pub fn with_link_loss_policy(mut self, vehicle: VehicleId, policy: LinkLossPolicy) -> Self {
        self.link_loss_overrides.push((vehicle, policy));
        self
    }
}
