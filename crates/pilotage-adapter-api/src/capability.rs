//! Capability model an adapter advertises to the session host (ADR-0008).

use pilotage_protocol::{
    ActionKind, IntentFamily, LogicalAxisId, ModeTarget, ReferenceFrame, ScopeId, VehicleId,
};

use crate::control::LinkLossPolicy;

/// Execution characteristics an adapter supports.
///
/// Plain booleans rather than a bitflags dependency: the set is small, fixed,
/// and read far more often than combined, so a struct keeps capability
/// checks self-documenting at call sites (`caps.execution.stepped` reads
/// clearly; a bitflag constant name would not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionMode {
    /// The adapter runs coupled to wall-clock time.
    pub real_time: bool,
    /// The adapter can be advanced by an explicit tick budget via `step`.
    pub stepped: bool,
    /// The adapter can run faster than wall-clock time.
    pub accelerated: bool,
    /// Identical inputs (including seed) produce bit-identical trajectories.
    pub deterministic: bool,
    /// The adapter can produce rendered video sources.
    pub render_capable: bool,
    /// The adapter drives a physical vehicle rather than a simulation.
    pub physically_embodied: bool,
}

/// One typed intent family a scope accepts, with the reference frames it
/// admits and its REAL magnitude limits — the numbers the adapter itself
/// enforces, so a client scaling sticks by these limits commands exactly the
/// envelope the vehicle flies (CTRL-01).
#[derive(Debug, Clone, PartialEq)]
pub struct IntentCapability {
    /// The intent family accepted.
    pub family: IntentFamily,
    /// The reference frames admitted for this family (empty for the
    /// frame-implicit families: body rate and gimbal rate).
    pub frames: Vec<ReferenceFrame>,
    /// Bound on the horizontal linear term (m/s for velocity, m for
    /// position hold). Zero means no bound advertised.
    pub max_linear: f32,
    /// Bound on the vertical linear term when the vehicle's vertical
    /// envelope is tighter; zero falls back to `max_linear`.
    pub max_vertical: f32,
    /// Bound on the angular term (rad/s for rates, rad for headings; the
    /// tilt angle for attitude-thrust). Zero means no bound advertised.
    pub max_angular: f32,
    /// For attitude-thrust only: the heading-setpoint slew rate (rad/s) a
    /// direct-flight client integrates its yaw stick at. Zero elsewhere.
    pub max_yaw_rate: f32,
}

impl IntentCapability {
    /// The effective vertical bound: `max_vertical`, falling back to
    /// `max_linear` when unset.
    #[must_use]
    pub fn effective_vertical(&self) -> f32 {
        if self.max_vertical > 0.0 {
            self.max_vertical
        } else {
            self.max_linear
        }
    }
}

/// One typed discrete action a scope accepts, with the mode targets it
/// admits for a mode request (empty for actions carrying no target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionCapability {
    /// The action kind accepted.
    pub action: ActionKind,
    /// The mode targets admitted for a `ModeRequest`; empty otherwise.
    pub mode_targets: Vec<ModeTarget>,
}

/// One legacy numeric axis routed onto a typed intent component: the axis
/// identifier and the sign applied to its `[-1, 1]` value before limit
/// scaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegacyAxisRoute {
    /// The legacy logical axis identifier.
    pub axis: u16,
    /// Sign applied to the normalized value (`1.0` or `-1.0`).
    pub sign: f32,
}

/// How a scope's LEGACY numeric payload translates into its typed command —
/// declared by the adapter as DATA, consumed by the session host's single
/// compatibility boundary. Adapters themselves consume ONLY typed commands;
/// no numeric axis or button identifier crosses the adapter boundary as
/// meaning (CTRL-01).
#[derive(Debug, Clone, PartialEq)]
pub enum LegacyCommandMap {
    /// The scope's numeric axes command a velocity: each mapped component is
    /// the signed normalized axis value scaled by the scope's advertised
    /// velocity limit, and the mapped buttons become typed actions.
    Velocity {
        /// Axis routed to forward velocity (`vx`).
        vx: Option<LegacyAxisRoute>,
        /// Axis routed to rightward velocity (`vy`).
        vy: Option<LegacyAxisRoute>,
        /// Axis routed to downward velocity (`vz`).
        vz: Option<LegacyAxisRoute>,
        /// Axis routed to yaw rate.
        yaw_rate: Option<LegacyAxisRoute>,
        /// Button whose pressed edge becomes `Arm`.
        arm_button: Option<u16>,
        /// Button whose pressed edge becomes `Disarm`.
        disarm_button: Option<u16>,
        /// Button whose pressed edge becomes `SimReset`.
        reset_button: Option<u16>,
    },
    /// The scope's numeric axes command a gimbal rate: pitch/yaw scaled by
    /// the advertised angular limit, and the mapped button recenters.
    GimbalRate {
        /// Axis routed to pitch rate.
        pitch: Option<LegacyAxisRoute>,
        /// Axis routed to yaw rate.
        yaw: Option<LegacyAxisRoute>,
        /// Button whose pressed edge becomes `GimbalRecenter`.
        recenter_button: Option<u16>,
    },
}

/// A control scope a vehicle exposes: the logical axes its LEGACY numeric
/// payload accepts, and the typed intents/actions it consumes (CTRL-01).
///
/// Scopes are host-published vocabulary (ADR-0006), so `scope` is the
/// string-backed `ScopeId` rather than a fixed enum.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeDescriptor {
    /// The control scope identifier (e.g. `"vehicle.motion"`).
    pub scope: ScopeId,
    /// Logical axes this scope accepts on the legacy numeric payload.
    pub axes: Vec<LogicalAxisId>,
    /// The typed intent families this scope accepts.
    pub intents: Vec<IntentCapability>,
    /// The typed discrete actions this scope accepts.
    pub actions: Vec<ActionCapability>,
    /// How the legacy numeric payload translates into this scope's typed
    /// command, when legacy senders are still admitted. `None` rejects
    /// legacy payloads for this scope outright.
    pub legacy: Option<LegacyCommandMap>,
}

/// A vehicle an adapter exposes, its control scopes, and the link-loss
/// actions it supports for that vehicle.
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleDescriptor {
    /// Identifies the vehicle.
    pub id: VehicleId,
    /// Control scopes assignable on this vehicle.
    pub scopes: Vec<ScopeDescriptor>,
    /// Link-loss actions this vehicle supports.
    pub link_loss_actions: Vec<LinkLossPolicy>,
}

/// The full capability description an adapter reports to the session host
/// (ADR-0008): vehicles present, execution characteristics, and adapter
/// version for compatibility negotiation.
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterCapabilities {
    /// Execution characteristics supported by this adapter.
    pub execution: ExecutionMode,
    /// Vehicles this adapter exposes.
    pub vehicles: Vec<VehicleDescriptor>,
    /// Adapter implementation version, for compatibility negotiation.
    pub adapter_version: String,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{AdapterCapabilities, ExecutionMode, ScopeDescriptor, VehicleDescriptor};
    use pilotage_protocol::{LogicalAxisId, ScopeId, VehicleId};

    #[test]
    fn execution_mode_default_is_all_false() {
        let mode = ExecutionMode::default();
        assert!(!mode.real_time);
        assert!(!mode.stepped);
        assert!(!mode.accelerated);
        assert!(!mode.deterministic);
        assert!(!mode.render_capable);
        assert!(!mode.physically_embodied);
    }

    #[test]
    fn capabilities_hold_declared_vehicles() {
        let caps = AdapterCapabilities {
            execution: ExecutionMode {
                stepped: true,
                deterministic: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: VehicleId::new(1),
                scopes: vec![ScopeDescriptor {
                    scope: ScopeId::new("vehicle.motion"),
                    axes: vec![LogicalAxisId::new(2), LogicalAxisId::new(3)],
                    intents: vec![],
                    actions: vec![],
                    legacy: None,
                }],
                link_loss_actions: vec![],
            }],
            adapter_version: "0.1.0".to_owned(),
        };
        assert_eq!(caps.vehicles.len(), 1);
        assert!(caps.execution.stepped);
    }
}
