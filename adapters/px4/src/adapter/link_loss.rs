//! Per-scope link-loss enactment (ADR-0008): latch the scope, then drive ONLY
//! that scope's actuation to its safe state. Motion neutralizes the FC velocity
//! setpoint; the gimbal stops its slew with a verified zero-rate. Neither scope
//! reaches the other, so a gimbal failsafe never brakes the aircraft and a
//! flight failsafe never freezes the camera.

use pilotage_adapter_api::{LinkLossEnactError, LinkLossPolicy};
use pilotage_protocol::{ScopeId, VehicleId};

use super::{FLIGHT_SCOPE, GIMBAL_SCOPE, Px4Adapter};

impl Px4Adapter {
    /// Sets or clears one scope's link-loss policy on the adapter. Latches the
    /// scope first (an unenactable engage still suppresses that scope's
    /// frames), then, on engagement, drives that scope's own actuation to its
    /// safe state. A refused actuation is a typed failure the host counts as a
    /// fail-closed fault — the latch stays engaged regardless.
    pub(super) fn enact_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        // Latch per-scope first: even an unenactable engage suppresses that
        // scope's frames, and another scope's latch is untouched.
        match &policy {
            Some(policy) => {
                self.link_loss_policy.insert(scope.clone(), *policy);
            }
            None => {
                self.link_loss_policy.remove(scope);
            }
        }
        // Clearing only removes the latch — the returning holder resumes
        // control. Engagement must actively drive THIS scope's actuation to
        // its safe state, and only its own: neutralizing motion brakes the
        // FC, neutralizing the gimbal stops the slew, and neither reaches the
        // other.
        if policy.is_none() {
            return Ok(());
        }
        match scope.as_str() {
            FLIGHT_SCOPE => {
                let Some(uplink) = self.uplink.as_mut() else {
                    return Err(LinkLossEnactError::NoActuationChannel);
                };
                let failures_before = uplink.send_failures();
                uplink.neutralize();
                if uplink.send_failures() != failures_before {
                    return Err(LinkLossEnactError::ChannelRejected {
                        detail: "the neutral setpoint send was refused".to_owned(),
                    });
                }
                Ok(())
            }
            GIMBAL_SCOPE => {
                // A gimbal failsafe QUEUES a zero-rate stop NOW, without ever
                // touching flight — an existing nonzero rate must not coast on
                // until the far slower stale-demand cutoff. This is best-effort:
                // `Ok` means the claim and zero-rate reached their lanes, not
                // that the FC confirmed a stop (the declared safety net is the
                // FC's own setpoint-timeout failsafe). A lane full/closed is a
                // typed fault the host counts.
                let Some(gimbal) = self.gimbal.as_mut() else {
                    return Err(LinkLossEnactError::NoActuationChannel);
                };
                if gimbal.queue_link_loss_stop() {
                    Ok(())
                } else {
                    Err(LinkLossEnactError::ChannelRejected {
                        detail: "the zero-rate gimbal setpoint could not be queued".to_owned(),
                    })
                }
            }
            // A scope with no independent actuation channel has nothing to
            // drive; the latch alone suppresses its frames.
            _ => Ok(()),
        }
    }
}
