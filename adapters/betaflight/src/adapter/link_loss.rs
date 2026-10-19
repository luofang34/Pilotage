//! Per-scope link-loss enactment (ADR-0008).
//!
//! Neutralizing the flight scope replaces the demand with a hold: the
//! outer loop brakes, captures position and altitude, and keeps the quad
//! there, which is what PX4's Hold does for its vehicles. The link keeps
//! streaming RC throughout, so Betaflight's own receiver-loss failsafe
//! (auto-land after its guard time) stays the independent net for the case
//! where this process or its link to the flight controller dies.

use std::time::Instant;

use pilotage_adapter_api::{LinkLossEnactError, LinkLossPolicy};
use pilotage_protocol::{ScopeId, VehicleId};

use super::{BetaflightAdapter, FLIGHT_SCOPE};
use crate::outer_loop::Demand;

impl BetaflightAdapter {
    /// Sets or clears one scope's link-loss policy. An engage records the
    /// latch before it acts, so a refused actuation still suppresses the
    /// scope; a clear drops the latch only on success.
    pub(super) fn enact_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        let Some(policy) = policy else {
            self.link_loss_policy.remove(scope);
            return Ok(());
        };
        self.link_loss_policy.insert(scope.clone(), policy);
        if scope.as_str() != FLIGHT_SCOPE {
            return Ok(());
        }
        if self.commands.is_closed() {
            return Err(LinkLossEnactError::NoActuationChannel);
        }
        self.commands.send_modify(|command| {
            command.demand = Demand::default();
            command.demand_at = Some(Instant::now());
        });
        Ok(())
    }
}
