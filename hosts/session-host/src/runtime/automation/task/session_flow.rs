//! The mission principal's session flow: bootstrap-stream replies
//! (welcome, lease, action results, rejections) and best-effort
//! datagrams (telemetry, rejection notices), decoded with the same wire
//! vocabulary remote clients read.

use pilotage_mission::{MissionEngine, MissionLimits};
use pilotage_protocol::{
    ControlAction, ControlActionResult, FrameRejected, FrameRejectionReason, LeaseRequest,
    LeaseResponse, ProfileActivation, ScopeId, ServerWelcome, wire,
};
use pilotage_session::DomainEnvelope;
use prost::Message;
use tracing::{debug, error, info, warn};

use crate::runtime::HOST_VEHICLE;

use super::nav_guidance::NavGuidancePublisher;
use super::{
    ACTIVATION_REVISION, MISSION_SCOPE, MissionTask, NO_DEVICE_DIGEST, PROFILE_DIGEST, PROFILE_ID,
    PROFILE_REVISION, ownship,
};

impl MissionTask {
    /// One reliable bootstrap-stream reply, decoded with the same
    /// envelope framing the remote clients read.
    pub(super) async fn on_bootstrap(&mut self, bytes: &[u8]) -> bool {
        let envelope = match pilotage_protocol::decode_envelope_length_delimited(bytes) {
            Ok((envelope, _rest)) => envelope,
            Err(error) => {
                warn!(%error, "undecodable bootstrap reply");
                return true;
            }
        };
        match envelope.payload {
            Some(wire::envelope::Payload::ServerWelcome(welcome)) => {
                match ServerWelcome::try_from(welcome) {
                    Ok(welcome) => self.on_welcome(welcome).await,
                    Err(error) => {
                        error!(%error, "welcome failed to convert");
                        false
                    }
                }
            }
            Some(wire::envelope::Payload::LeaseResponse(response)) => {
                match LeaseResponse::try_from(response) {
                    Ok(response) => self.on_lease_response(&response),
                    Err(error) => warn!(%error, "lease response failed to convert"),
                }
                true
            }
            Some(wire::envelope::Payload::ControlActionResult(result)) => {
                match ControlActionResult::try_from(result) {
                    Ok(result) => self.on_action_result(&result),
                    Err(error) => warn!(%error, "action result failed to convert"),
                }
                true
            }
            Some(wire::envelope::Payload::FrameRejected(rejection)) => {
                match FrameRejected::try_from(rejection) {
                    Ok(rejection) => self.on_frame_rejected(&rejection),
                    Err(error) => warn!(%error, "frame rejection failed to convert"),
                }
                true
            }
            Some(wire::envelope::Payload::LeaseReleased(released)) => {
                debug!(?released, "lease release acknowledged");
                true
            }
            _ => true,
        }
    }

    /// The welcome closes the handshake: capture identities, tighten the
    /// mission ceilings to the advertised envelope, build the flight
    /// engine, then announce the profile and request the motion lease.
    async fn on_welcome(&mut self, welcome: ServerWelcome) -> bool {
        // Fencing is permanent for this task: even a fresh welcome must
        // not restart activation or re-lease the motion scope.
        if self.fenced {
            return false;
        }
        self.session = Some(welcome.session);
        self.principal = Some(welcome.principal);
        self.nav_guidance = Some(NavGuidancePublisher::for_session(welcome.session));
        self.update(|status| status.session = Some(welcome.session.as_u64()));
        let Some(plan) = self.plan.take() else {
            return true;
        };
        let mut config = plan.config();
        if !tighten_limits(&mut config.limits, &welcome.host_capabilities) {
            // Streaming velocity intents the host never advertised would
            // fail open against the command gate; a mission with no
            // authorized envelope does not fly.
            self.fence("no vehicle.motion velocity capability advertised");
            return false;
        }
        let limits = config.limits;
        match MissionEngine::new(
            &plan.navdata.snapshot,
            plan.navdata.provenance.clone(),
            config,
        ) {
            Ok((engine, record)) => {
                info!(
                    route = %record.route_input,
                    max_horizontal_mps = limits.max_horizontal_mps,
                    max_vertical_mps = limits.max_vertical_mps,
                    max_yaw_rate_rps = limits.max_yaw_rate_rps,
                    "mission engine ready under advertised limits"
                );
                self.mission = Some(engine);
            }
            Err(error) => {
                error!(%error, "mission engine failed to build");
                return false;
            }
        }
        let activation = DomainEnvelope::ProfileActivation(ProfileActivation {
            session: welcome.session,
            profile_id: PROFILE_ID.to_owned(),
            profile_revision: PROFILE_REVISION,
            activation_revision: ACTIVATION_REVISION,
            digest: PROFILE_DIGEST,
            device_profile_id: String::new(),
            device_profile_revision: 0,
            device_digest: NO_DEVICE_DIGEST,
        });
        if !self.send_message(activation).await {
            return false;
        }
        self.update(|status| status.activation_sent = true);
        let lease = DomainEnvelope::Lease(LeaseRequest {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
        });
        self.send_message(lease).await
    }

    fn on_lease_response(&mut self, response: &LeaseResponse) {
        if response.vehicle != HOST_VEHICLE || response.scope.as_str() != MISSION_SCOPE {
            return;
        }
        if response.granted {
            info!(
                generation = response.generation.as_u64(),
                "mission motion lease granted"
            );
            self.generation = Some(response.generation);
            self.update(|status| status.lease_generation = Some(response.generation.as_u64()));
        } else {
            error!(reason = ?response.reason, "mission motion lease denied");
            self.fence("the motion lease was denied");
        }
    }

    /// Routes a correlated action result back into the mission engine.
    fn on_action_result(&mut self, result: &ControlActionResult) {
        let Some(mission_id) = self.pending_actions.remove(&result.action_id) else {
            debug!(action_id = result.action_id, "uncorrelated action result");
            return;
        };
        info!(
            action = ?result.action,
            accepted = result.accepted,
            detail = %result.detail,
            "mission action result"
        );
        if let Some(mission) = self.mission.as_mut() {
            mission.on_action_result(mission_id, result.accepted);
        }
        if result.accepted && matches!(result.action, ControlAction::Arm) {
            self.update(|status| status.arm_accepted = true);
        }
    }

    fn on_frame_rejected(&mut self, rejection: &FrameRejected) {
        warn!(
            reason = ?rejection.reason,
            sequence = rejection.sequence.as_u32(),
            current_generation = rejection.current_generation.as_u64(),
            "mission frame rejected"
        );
        self.update(|status| status.frames_rejected = status.frames_rejected.wrapping_add(1));
        // A stale-generation or no-holder rejection means authority moved
        // on while an event was still in flight; stop framing rather than
        // keep hammering a superseded grant.
        if matches!(
            rejection.reason,
            FrameRejectionReason::StaleGeneration | FrameRejectionReason::NoHolder
        ) {
            self.fence("frames are rejected under a superseded authority");
        }
    }

    /// One best-effort datagram: telemetry feeding the mission's ownship
    /// path, or a frame-rejection notice.
    pub(super) fn on_datagram(&mut self, bytes: &[u8]) {
        let Ok(envelope) = wire::Envelope::decode(bytes) else {
            debug!("undecodable datagram payload");
            return;
        };
        match envelope.payload {
            Some(wire::envelope::Payload::TelemetrySample(sample)) => self.on_telemetry(&sample),
            Some(wire::envelope::Payload::FrameRejected(rejection)) => {
                match FrameRejected::try_from(rejection) {
                    Ok(rejection) => self.on_frame_rejected(&rejection),
                    Err(error) => warn!(%error, "frame rejection failed to convert"),
                }
            }
            _ => {}
        }
    }

    fn on_telemetry(&mut self, sample: &wire::TelemetrySample) {
        let now = self.mission_now();
        let Some(mission) = self.mission.as_mut() else {
            return;
        };
        let ours = sample
            .vehicle
            .as_ref()
            .is_some_and(|vehicle| vehicle.value == HOST_VEHICLE.as_u64());
        if !ours {
            return;
        }
        if let Some(ownship) = ownship::ownship_from_wire(sample, now, self.planar_pose_is_truth) {
            mission.on_ownship(&ownship, now);
        }
    }
}

/// Tightens the mission ceilings to the advertised `vehicle.motion`
/// velocity envelope: a nonzero advertised bound lowers the configured
/// ceiling; a zero advertisement means "no bound" on the wire and leaves
/// the configured ceiling in force. Vertical mirrors the command gate's
/// effective bound (`max_vertical`, falling back to `max_linear`).
/// Returns false when no velocity capability is advertised at all — the
/// mission has no authorized envelope and must not fly.
fn tighten_limits(limits: &mut MissionLimits, capabilities: &wire::HostCapabilities) -> bool {
    let Some(intent) = velocity_capability(capabilities) else {
        error!("no vehicle.motion velocity capability advertised; the mission cannot fly");
        return false;
    };
    tighten(&mut limits.max_horizontal_mps, intent.max_linear);
    let vertical = if intent.max_vertical > 0.0 {
        intent.max_vertical
    } else {
        intent.max_linear
    };
    tighten(&mut limits.max_vertical_mps, vertical);
    tighten(&mut limits.max_yaw_rate_rps, intent.max_angular);
    true
}

fn tighten(limit: &mut f64, advertised: f32) {
    if advertised > 0.0 {
        *limit = limit.min(f64::from(advertised));
    }
}

/// The advertised velocity capability for this principal's (vehicle,
/// scope), if the welcome carried one.
fn velocity_capability(capabilities: &wire::HostCapabilities) -> Option<&wire::IntentCapability> {
    capabilities
        .vehicles
        .iter()
        .find(|vehicle| {
            vehicle
                .vehicle
                .as_ref()
                .is_some_and(|id| id.value == HOST_VEHICLE.as_u64())
        })?
        .scopes
        .iter()
        .find(|scope| {
            scope
                .scope
                .as_ref()
                .is_some_and(|id| id.value == MISSION_SCOPE)
        })?
        .intents
        .iter()
        .find(|intent| intent.family == wire::IntentFamily::Velocity as i32)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    fn advertised(max_linear: f32) -> wire::HostCapabilities {
        wire::HostCapabilities {
            vehicles: vec![wire::VehicleDescriptor {
                vehicle: Some(wire::VehicleId {
                    value: HOST_VEHICLE.as_u64(),
                }),
                scopes: vec![wire::ScopeDescriptor {
                    scope: Some(wire::ScopeId {
                        value: MISSION_SCOPE.to_owned(),
                    }),
                    intents: vec![wire::IntentCapability {
                        family: wire::IntentFamily::Velocity as i32,
                        max_linear,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_velocity_capability_is_mission_fatal_not_fail_open() {
        let mut limits = MissionLimits::default();
        let configured = limits;
        assert!(!tighten_limits(
            &mut limits,
            &wire::HostCapabilities::default()
        ));
        assert_eq!(
            limits, configured,
            "refusal, not silent flight on unadvertised ceilings"
        );
    }

    #[test]
    fn an_advertised_envelope_tightens_the_configured_ceilings() {
        let mut limits = MissionLimits::default();
        assert!(tighten_limits(&mut limits, &advertised(1.0)));
        assert_eq!(limits.max_horizontal_mps, 1.0);
    }
}
