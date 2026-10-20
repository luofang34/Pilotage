//! Per-scope link-loss enactment (ADR-0008): latch the scope, then drive
//! the flight scope to its safe state — a zero velocity setpoint, then
//! silence, which ArduCopter's GUIDED timeout turns into a hover.

use pilotage_adapter_api::{LinkLossEnactError, LinkLossPolicy};
use pilotage_protocol::{ScopeId, VehicleId};

use super::{ArduPilotAdapter, DIRECT_SCOPE, FLIGHT_SCOPE, GIMBAL_SCOPE};

impl ArduPilotAdapter {
    /// Sets or clears one scope's link-loss policy. An engage records the
    /// latch before actuation, so a refused actuation still suppresses the
    /// scope; a clear drops the latch only on the success path.
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
        if let Some(camera) = self.camera.as_mut() {
            camera.invalidate_pending_commands(vehicle, scope);
        }
        if scope.as_str() == GIMBAL_SCOPE {
            let Some(gimbal) = self.gimbal.as_mut() else {
                return Err(LinkLossEnactError::NoActuationChannel);
            };
            return if gimbal.queue_link_loss_stop() {
                Ok(())
            } else {
                Err(LinkLossEnactError::ChannelRejected {
                    detail: "the gimbal stop could not be queued".to_owned(),
                })
            };
        }
        if scope.as_str() != FLIGHT_SCOPE && scope.as_str() != DIRECT_SCOPE {
            return Ok(());
        }
        let Some(uplink) = self.uplink.as_mut() else {
            return Err(LinkLossEnactError::NoActuationChannel);
        };
        let failures_before = uplink.send_failures();
        uplink.neutralize();
        if uplink.send_failures() != failures_before {
            return Err(LinkLossEnactError::ChannelRejected {
                detail: "the zero velocity setpoint could not be published".to_owned(),
            });
        }
        Ok(())
    }
}
