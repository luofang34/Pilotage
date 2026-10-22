//! The mission principal's session flow: bootstrap-stream replies
//! (welcome, lease, action results, rejections) and best-effort
//! datagrams (telemetry, rejection notices), decoded with the same wire
//! vocabulary remote clients read.

use pilotage_mission::{MissionEngine, MissionLimits, OwnshipSource};
use pilotage_protocol::{
    ControlAction, ControlActionResult, FrameRejected, FrameRejectionReason, LeaseRequest,
    LeaseResponse, ProfileActivation, ScopeId, ServerWelcome, wire,
};
use pilotage_session::DomainEnvelope;
use prost::Message;
use tracing::{debug, error, info, warn};

use crate::runtime::HOST_VEHICLE;

use super::super::PlanSource;
use super::nav_guidance::NavGuidancePublisher;
use super::{
    ACTIVATION_REVISION, MISSION_SCOPE, MissionTask, NO_DEVICE_DIGEST, PROFILE_DIGEST, PROFILE_ID,
    PROFILE_REVISION, ownship,
};

/// Height above the launch point, metres, from which a real vehicle counts
/// as launched. A DJI native takeoff hovers at about 1.2 m; below this the
/// vehicle is on the ground and refuses velocity control.
pub(super) const AIRBORNE_HEIGHT_M: f64 = 1.0;
/// How long a held, airborne mission waits before it asks for the motion
/// lease again.
pub(super) const LEASE_RETRY_NS: u64 = 5_000_000_000;

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
        let built = match &plan.source {
            PlanSource::Navdata(navdata) => {
                MissionEngine::new(&navdata.snapshot, navdata.provenance.clone(), config).map(
                    |(engine, record)| {
                        info!(route = %record.route_input, "mission plan is a route");
                        engine
                    },
                )
            }
            PlanSource::Survey(request) => {
                MissionEngine::from_survey(request, config).map(|(engine, record)| {
                    info!(
                        lines = record.line_count,
                        waypoints = record.waypoint_count,
                        "mission plan is an area survey"
                    );
                    engine
                })
            }
        };
        match built {
            Ok(engine) => {
                info!(
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
        if self.real_vehicle {
            self.wait_for_launch();
            return true;
        }
        self.request_lease().await
    }

    /// Asks for the motion lease.
    pub(super) async fn request_lease(&mut self) -> bool {
        let lease = DomainEnvelope::Lease(LeaseRequest {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
        });
        self.send_message(lease).await
    }

    /// Holds a real vehicle's mission until its pilot has launched it: the
    /// mission does not ask for the motion lease on the ground, where the
    /// vehicle would refuse velocity control, and its phase clocks do not
    /// run (ADR-0051).
    pub(super) fn wait_for_launch(&mut self) {
        self.awaiting_launch = true;
        self.hold("waiting for the pilot to launch the aircraft");
        self.update(|status| status.awaiting_launch = true);
    }

    /// Notes the vehicle airborne: the next tick asks for the lease, and
    /// the mission keeps asking while another principal holds the scope.
    fn on_airborne(&mut self, height_m: f64) {
        self.awaiting_launch = false;
        self.lease_due = true;
        self.lease_wanted = true;
        info!(
            height_m,
            "the aircraft is airborne; the mission asks for the motion lease"
        );
        self.update(|status| status.awaiting_launch = false);
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
            self.take_generation(response.generation);
        } else if self.lease_wanted {
            // A human holds the scope after the launch: the mission holds
            // and asks again until the human releases or hands it over.
            debug!(reason = ?response.reason, "mission motion lease denied; retrying");
            self.hold("the motion lease is held by another principal");
            self.schedule_lease_retry();
        } else {
            // A human already holds the scope: the mission waits for a
            // handover instead of ending (ADR-0051).
            error!(reason = ?response.reason, "mission motion lease denied");
            self.hold("the motion lease was denied; waiting for a handover");
        }
    }

    /// Arms the next lease request `LEASE_RETRY_NS` from now.
    pub(super) fn schedule_lease_retry(&mut self) {
        if self.lease_wanted && !self.fenced {
            self.lease_retry_at =
                Some(self.mission_now().as_nanos().saturating_add(LEASE_RETRY_NS));
            self.update(|status| status.lease_retries = status.lease_retries.wrapping_add(1));
        }
    }

    /// Whether a held mission's retry has come due at `now` on the mission clock.
    pub(super) fn lease_retry_due(&self, now: u64) -> bool {
        self.held
            && !self.fenced
            && !self.awaiting_launch
            && self.lease_retry_at.is_some_and(|at| now >= at)
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
        let ownship = match self.ownship_source {
            OwnshipSource::SimulationTruth => {
                ownship::ownship_from_wire(sample, now, self.planar_pose_is_truth)
            }
            OwnshipSource::VehicleEstimate { .. } => {
                ownship::estimate_from_wire(sample, now, |latitude_rad, longitude_rad, altitude| {
                    mission.ned_of(latitude_rad, longitude_rad, altitude)
                })
            }
        };
        let Some(ownship) = ownship else {
            return;
        };
        mission.on_ownship(&ownship, now);
        let height_m = -ownship.ned[2];
        if self.awaiting_launch && height_m >= AIRBORNE_HEIGHT_M {
            self.on_airborne(height_m);
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
mod tests;
