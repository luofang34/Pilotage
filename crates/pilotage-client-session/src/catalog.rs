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
    /// Present holders at the moment of admission, so ownership renders
    /// without waiting for the next authority event.
    pub scope_holders: Vec<wire::ScopeHolderSnapshot>,
}

/// One vehicle and the scopes it publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleCatalog {
    /// The vehicle's identity.
    pub vehicle_id: u64,
    /// The host's display name for the vehicle.
    pub display_name: String,
    /// The control scopes this vehicle publishes.
    pub scopes: Vec<ScopeCatalog>,
}

/// One control scope, with the typed capabilities the host advertised.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeCatalog {
    /// The scope's identity string.
    pub scope: String,
    /// The typed intent families this scope accepts. Empty on a scope that
    /// takes only the legacy numeric payload.
    pub intents: Vec<wire::IntentCapability>,
    /// The typed discrete actions this scope accepts.
    pub actions: Vec<wire::ActionCapability>,
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
                scopes: descriptor
                    .scopes
                    .iter()
                    .map(|scope| ScopeCatalog {
                        scope: scope
                            .scope
                            .as_ref()
                            .map(|s| s.value.clone())
                            .unwrap_or_default(),
                        intents: scope.intents.clone(),
                        actions: scope.actions.clone(),
                    })
                    .collect(),
            })
            .collect();
        Some(Self {
            session_id,
            principal_id,
            host_version: capabilities.host_version,
            vehicles,
            scope_holders: welcome.scope_holders.clone(),
        })
    }

    /// Whether the host offers any control scope at all. Control further
    /// requires a granted lease; this is the "offered inputs" leg of the
    /// ADR-0037 availability rule, not an authorization.
    #[must_use]
    pub fn offers_control(&self) -> bool {
        self.vehicles.iter().any(|v| !v.scopes.is_empty())
    }
}
