//! Capability model an adapter advertises to the session host (ADR-0008).

use pilotage_protocol::{LogicalAxisId, ScopeId, VehicleId};

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

/// A control scope a vehicle exposes, and the logical axes it accepts.
///
/// Scopes are host-published vocabulary (ADR-0006), so `scope` is the
/// string-backed `ScopeId` rather than a fixed enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDescriptor {
    /// The control scope identifier (e.g. `"vehicle.motion"`).
    pub scope: ScopeId,
    /// Logical axes this scope accepts.
    pub axes: Vec<LogicalAxisId>,
}

/// A vehicle an adapter exposes, its control scopes, and the link-loss
/// actions it supports for that vehicle.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
                }],
                link_loss_actions: vec![],
            }],
            adapter_version: "0.1.0".to_owned(),
        };
        assert_eq!(caps.vehicles.len(), 1);
        assert!(caps.execution.stepped);
    }
}
