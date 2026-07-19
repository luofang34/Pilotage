//! The PX4 `VehicleAdapter`: telemetry sampling from the shared
//! MAVLink link and offboard flight control with the same gate
//! discipline as the Aviate adapter (link-loss latch, commanded-reset
//! latch, disarm always allowed).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch,
    VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::VehicleId;
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, ScopeId, ScopedControlFrame};
use pilotage_timing::SimTick;

use pilotage_mavlink::{AuthorizationSource, LinkConfig, LinkState, MavlinkLink};

use crate::error::Px4AdapterError;
use crate::uplink::Px4Uplink;

mod control;
mod sampling;
#[cfg(test)]
mod tests;

use control::rejected_control;

/// The control scope: four canonical flight axes as velocity demands.
pub const FLIGHT_SCOPE: &str = "vehicle.motion";
/// Canonical `roll` axis (0): lateral velocity, + = right.
pub const ROLL_AXIS: u16 = 0;
/// Canonical `pitch` axis (1): forward velocity, + = forward.
pub const PITCH_AXIS: u16 = 1;
/// Canonical `throttle` axis (2): climb rate, + = climb.
pub const THROTTLE_AXIS: u16 = 2;
/// Canonical `yaw` axis (3): yaw rate, + = clockwise.
pub const YAW_AXIS: u16 = 3;
/// Logical button whose press arms the vehicle.
pub const ARM_BUTTON: u16 = 0;
/// Logical button whose press disarms the vehicle.
pub const DISARM_BUTTON: u16 = 1;
/// Logical button whose press resets the simulation.
pub const RESET_BUTTON: u16 = 2;

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever.
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Telemetry-only-until-armed adapter for PX4 (ADR-0018). Real-time
/// (ADR-0013): PX4 advances on its own clock; `step` reports the
/// latest observed source time as the simulation tick.
#[derive(Debug)]
pub struct Px4Adapter {
    vehicle: VehicleId,
    // The shared receive-link cache. PX4's standard ESTIMATOR_STATUS
    // is the authorization source (LINK-04: the estimate is the only
    // basis for state-dependent control; there is no truth oracle).
    estimate: Option<EstimateSource>,
    uplink: Option<Px4Uplink>,
    last_reset: Option<std::time::Instant>,
    // Commanded-reset latch: engaged when a sim reset is requested,
    // cleared only by a fresh estimate source epoch plus demonstrated
    // neutral input. While engaged, everything except disarm is
    // rejected.
    reset_latch: Option<control::ResetLatch>,
    // Reset script spawns recorded instead of executed, so tests can
    // press the reset button without touching a live simulator.
    #[cfg(test)]
    reset_spawns: u32,
    link_loss_policy: Option<LinkLossPolicy>,
}

/// The MAVLink estimate source: the shared cache plus the link task
/// keeping it fed (dropped together).
#[derive(Debug)]
struct EstimateSource {
    state: Arc<Mutex<LinkState>>,
    _link: Option<MavlinkLink>,
}

impl Px4Adapter {
    /// Binds the MAVLink receive link (`PILOTAGE_PX4_ADDR`, default
    /// `127.0.0.1:14550` — PX4 streams attitude, local position, and
    /// the estimator status on its GCS instance) and the offboard
    /// command uplink. A failed uplink bind degrades to
    /// telemetry-only rather than failing the adapter.
    ///
    /// # Errors
    ///
    /// Returns [`Px4AdapterError::Link`] when the receive link cannot
    /// bind any socket.
    pub async fn start(vehicle: VehicleId) -> Result<Self, Px4AdapterError> {
        let config = link_config();
        let incarnation = pilotage_adapter_api::SourceIncarnation::new(rand_incarnation());
        let link = MavlinkLink::start(config, incarnation).await?;
        let state = link.state();
        let uplink = match Px4Uplink::new() {
            Ok(mut uplink) => {
                uplink.set_expected_source(config.system_id, config.component_id);
                Some(uplink)
            }
            Err(error) => {
                tracing::warn!(%error, "PX4 uplink unavailable; telemetry-only");
                None
            }
        };
        Ok(Self {
            vehicle,
            estimate: Some(EstimateSource {
                state,
                _link: Some(link),
            }),
            uplink,
            last_reset: None,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            link_loss_policy: None,
        })
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LinkState>>) -> Self {
        Self {
            vehicle,
            estimate: Some(EstimateSource { state, _link: None }),
            uplink: None,
            last_reset: None,
            reset_latch: None,
            reset_spawns: 0,
            link_loss_policy: None,
        }
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: Px4Uplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    fn validate_flight_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != FLIGHT_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        let known = [
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ];
        for (axis, _) in &frame.payload.axes {
            if !known.contains(axis) {
                return Err(RejectReason::UnknownAxis);
            }
        }
        Ok(())
    }
}

fn link_config() -> LinkConfig {
    let endpoint = std::env::var("PILOTAGE_PX4_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 14_550)));
    LinkConfig {
        endpoint,
        authorization_source: AuthorizationSource::StandardEstimatorStatus,
        // PX4 streams only ~15-20 s after boot (logger, EKF, and the
        // message-interval negotiation), so its post-restart clock is
        // far above the default reset-candidate ceiling.
        reset_candidate_max_ms: 60_000,
        ..LinkConfig::simulator()
    }
}

/// A random attachment identity; each adapter start is a distinct
/// source incarnation.
fn rand_incarnation() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    bytes[..8].copy_from_slice(&now.as_nanos().to_le_bytes()[..8]);
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes
}

impl VehicleAdapter for Px4Adapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: if self.uplink.is_some() {
                    vec![ScopeDescriptor {
                        scope: ScopeId::new(FLIGHT_SCOPE),
                        axes: vec![
                            LogicalAxisId::new(ROLL_AXIS),
                            LogicalAxisId::new(PITCH_AXIS),
                            LogicalAxisId::new(THROTTLE_AXIS),
                            LogicalAxisId::new(YAW_AXIS),
                        ],
                    }]
                } else {
                    vec![]
                },
                link_loss_actions: if self.uplink.is_some() {
                    vec![LinkLossPolicy::Neutralize]
                } else {
                    vec![]
                },
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        if let Some(outcome) = self.gated_flight_outcome(frame, tick) {
            return outcome;
        }
        let Some((current_yaw, _pos, _vel)) = self.current_pose() else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };
        for (button, edge) in &frame.payload.edges {
            if *edge != ButtonEdge::Pressed {
                continue;
            }
            if *button == LogicalButtonId::new(ARM_BUTTON) {
                uplink.begin_arm(current_yaw);
            }
        }
        let (sticks, transformed) = control::normalized_flight_sticks(frame);
        uplink.send_stick_frame(
            sticks[usize::from(ROLL_AXIS)],
            sticks[usize::from(PITCH_AXIS)],
            sticks[usize::from(THROTTLE_AXIS)],
            sticks[usize::from(YAW_AXIS)],
        );
        ApplyOutcome {
            tick,
            disposition: if transformed {
                Disposition::Transformed
            } else {
                Disposition::Accepted
            },
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        let batch = self
            .estimate
            .as_ref()
            .map(|source| sampling::mavlink_batch(self.vehicle, &source.state))
            .unwrap_or_default();
        let telemetry_flowing = batch
            .samples
            .first()
            .is_some_and(|sample| sample.avionics.is_some());
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.maintain(telemetry_flowing);
        }
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        vec![]
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        match policy {
            Some(_) => {
                let Some(uplink) = self.uplink.as_mut() else {
                    self.link_loss_policy = policy;
                    return Err(LinkLossEnactError::NoActuationChannel);
                };
                let failures_before = uplink.send_failures();
                uplink.neutralize();
                let refused = uplink.send_failures() != failures_before;
                self.link_loss_policy = policy;
                if refused {
                    return Err(LinkLossEnactError::ChannelRejected {
                        detail: "the neutral setpoint send was refused".to_owned(),
                    });
                }
                Ok(())
            }
            None => {
                self.link_loss_policy = None;
                Ok(())
            }
        }
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        let tick = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
            .and_then(|latest| latest.kinematics)
            .map_or(0, |kin| u64::from(kin.time_boot_ms).wrapping_mul(1_000_000));
        StepOutcome {
            advanced: 0,
            now: SimTick::new(tick),
        }
    }
}
