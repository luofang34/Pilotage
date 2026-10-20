//! The typed view of what an admitted session offers.
//!
//! Modules decide availability from this catalog rather than from a coarse
//! profile name (ADR-0037). The catalog is built from the host's own
//! `ServerWelcome`; nothing here is guessed client-side.

use pilotage_protocol::wire;

/// Everything the host stated at admission.
#[derive(Debug, Clone, PartialEq)]
pub struct Admission {
    /// The session the host assigned.
    pub session_id: u64,
    /// The principal identity the host assigned to this connection.
    pub principal_id: u64,
    /// The host's version string, for diagnostics.
    pub host_version: String,
    /// Every vehicle the host offers, with its control scopes.
    pub vehicles: Vec<VehicleCatalog>,
    /// Active control-feel identity, when the host has one.
    pub control_feel: Option<wire::ControlFeelIdentity>,
    /// Present holders at the moment of admission, so ownership renders
    /// without waiting for the next authority event.
    pub scope_holders: Vec<wire::ScopeHolderSnapshot>,
    /// Whether the host drives a physical vehicle rather than a simulation.
    pub physically_embodied: bool,
}

/// One vehicle and the scopes it publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleCatalog {
    /// The vehicle's identity.
    pub vehicle_id: u64,
    /// The host's display name for the vehicle.
    pub display_name: String,
    /// The kind of vehicle the adapter declares; `None` when it declared
    /// no class this build knows.
    pub vehicle_class: Option<pilotage_protocol::VehicleClass>,
    /// Which north the vehicle's attitude yaw uses, as the host knew it at
    /// admission; `None` when it was not stated. The vehicle's stamped
    /// magnetic variation ([`crate::telemetry::magnetic_variation`]) is the live
    /// statement and wins when the two disagree.
    pub heading_reference: Option<pilotage_protocol::HeadingReference>,
    /// The control scopes this vehicle publishes.
    pub scopes: Vec<ScopeCatalog>,
}

/// One control scope, with the typed capabilities the host advertised.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeCatalog {
    /// Payload operations and feedback declared by the host.
    pub payload: Option<wire::PayloadCapabilities>,
    /// The scope's identity string.
    pub scope: String,
    /// The typed intent families this scope accepts. Empty on a scope that
    /// takes only the legacy numeric payload.
    pub intents: Vec<wire::IntentCapability>,
    /// The typed discrete actions this scope accepts.
    pub actions: Vec<wire::ActionCapability>,
    /// The exclusive-authority group the host holds this scope under; the
    /// scope's own name when it is its own group. Authority events name
    /// the group, so a group revocation also ends this scope's lease.
    pub authority_group: String,
}

impl Admission {
    /// Builds the typed catalog from the host's welcome, or `None` when the
    /// welcome is structurally incomplete (a session with no identity is not
    /// an admission).
    #[must_use]
    pub fn from_welcome(welcome: &wire::ServerWelcome) -> Option<Self> {
        let session_id = welcome.session.as_ref()?.value;
        let principal_id = welcome.principal.as_ref().map_or(0, |p| p.value);
        let capabilities = welcome.host_capabilities.clone().unwrap_or_default();
        let vehicles = capabilities
            .vehicles
            .iter()
            .map(|descriptor| VehicleCatalog {
                vehicle_id: descriptor.vehicle.as_ref().map_or(0, |v| v.value),
                display_name: descriptor.display_name.clone(),
                vehicle_class: pilotage_protocol::VehicleClass::from_wire(descriptor.vehicle_class),
                heading_reference: pilotage_protocol::HeadingReference::from_wire(
                    descriptor.heading_reference,
                ),
                scopes: descriptor
                    .scopes
                    .iter()
                    .map(|scope| ScopeCatalog {
                        payload: scope.payload.clone(),
                        scope: scope
                            .scope
                            .as_ref()
                            .map(|s| s.value.clone())
                            .unwrap_or_default(),
                        intents: scope.intents.clone(),
                        actions: scope.actions.clone(),
                        authority_group: if scope.authority_group.is_empty() {
                            scope
                                .scope
                                .as_ref()
                                .map(|s| s.value.clone())
                                .unwrap_or_default()
                        } else {
                            scope.authority_group.clone()
                        },
                    })
                    .collect(),
            })
            .collect();
        Some(Self {
            session_id,
            principal_id,
            host_version: capabilities.host_version,
            vehicles,
            control_feel: capabilities.control_feel,
            scope_holders: welcome.scope_holders.clone(),
            physically_embodied: capabilities.physically_embodied,
        })
    }

    /// The authority group `scope` of `vehicle_id` is held under: the
    /// scope itself when the host declared no group or does not know it.
    #[must_use]
    pub fn authority_group_of<'a>(&'a self, vehicle_id: u64, scope: &'a str) -> &'a str {
        self.vehicles
            .iter()
            .find(|vehicle| vehicle.vehicle_id == vehicle_id)
            .and_then(|vehicle| vehicle.scopes.iter().find(|s| s.scope == scope))
            .map_or(scope, |s| s.authority_group.as_str())
    }

    /// Whether the host offers any control scope at all. Control further
    /// requires a granted lease; this is the "offered inputs" leg of the
    /// ADR-0037 availability rule, not an authorization.
    #[must_use]
    pub fn offers_control(&self) -> bool {
        self.vehicles.iter().any(|v| !v.scopes.is_empty())
    }
}
