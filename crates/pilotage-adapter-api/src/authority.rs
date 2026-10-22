//! Device-side loss of a specific software control grant.

use pilotage_protocol::{Generation, ScopeId, SessionId, VehicleId};

/// A grant the device no longer permits. This does not establish who controls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlAuthorityLoss {
    /// Vehicle whose device authority changed.
    pub vehicle: VehicleId,
    /// Concrete control scope used by the revoked grant.
    pub scope: ScopeId,
    /// Session whose commands must stop.
    pub session: SessionId,
    /// Grant generation whose commands must stop.
    pub generation: Generation,
}
