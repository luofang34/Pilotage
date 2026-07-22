//! The PX4 `VehicleAdapter`: telemetry sampling from the shared
//! MAVLink link and offboard flight control with the same gate
//! discipline as the Aviate adapter (link-loss latch followed by the
//! commanded-reset latch).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    ActionResult, AdapterCapabilities, ApplyOutcome, Disposition, LinkLossEnactError,
    LinkLossPolicy, RejectReason, StepBudget, StepOutcome, TelemetryBatch, TelemetrySample,
    VehicleAdapter, VideoSource,
};
use pilotage_protocol::VehicleId;
use std::collections::BTreeMap;

use pilotage_protocol::{ActionKind, LogicalAxisId, ScopeId, ScopedControlFrame};
use pilotage_timing::SimTick;

use pilotage_mavlink::{LinkState, MavlinkLink};

use crate::config::Px4Config;
use crate::error::Px4AdapterError;
use crate::uplink::{Px4Uplink, StickFrameDisposition};

mod advertisement;
mod camera;
mod control;
#[cfg(test)]
mod gimbal_link_loss_tests;
#[cfg(test)]
mod gimbal_tests;
mod link_loss;
mod pointing;
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
/// The gimbal pointing scope (GIM-01, ADR-0006 vocabulary): pitch/yaw
/// LOS rate demands, leased and fenced independently of flight.
pub const GIMBAL_SCOPE: &str = "vehicle.gimbal";
/// Gimbal-scope button whose press recenters the gimbal.
pub const GIMBAL_NEUTRAL_BUTTON: u16 = 0;

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
    // Gimbal-manager command path; rides the receive link's socket
    // because the FC's GCS instance retargets its telemetry stream to
    // the last peer that spoke.
    gimbal: Option<crate::gimbal::Px4GimbalControl>,
    // Pilotage's gz sidecar bridges the flight-deck rig's camera topics;
    // the adapter remains usable without video when it cannot spawn.
    frames: Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
    _camera_bridge: Option<pilotage_adapter_gazebo::BridgeClient>,
    _frame_forwarder: Option<tokio::task::JoinHandle<()>>,
    // Latest heartbeat-reported arm state; re-acquired per heartbeat so
    // its freshness honestly tracks the FC's liveness.
    arm: Option<ArmReport>,
    last_seen_heartbeat: Option<std::time::Instant>,
    arm_incarnation: pilotage_adapter_api::SourceIncarnation,
    // Payload-device stamp discipline for the gimbal lane: a stable
    // gimbal identity plus a reboot-aware epoch, kept apart from the FC
    // arm incarnation so a device reboot never regresses time under a
    // fixed identity+epoch.
    gimbal_stamp: sampling::GimbalStamp,
    started_at: std::time::Instant,
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
    // Per-scope link-loss latch (ADR-0008): engaging the gimbal scope must
    // not suppress or neutralize motion, and vice versa.
    link_loss_policy: BTreeMap<ScopeId, LinkLossPolicy>,
}

/// The MAVLink estimate source: the shared cache plus the link task
/// keeping it fed (dropped together).
#[derive(Debug)]
struct EstimateSource {
    state: Arc<Mutex<LinkState>>,
    _link: Option<MavlinkLink>,
}

/// The latest FC arm report derived from telemetry heartbeats.
#[derive(Debug, Clone, Copy)]
struct ArmReport {
    armed: bool,
    sequence: u32,
    acquired_at: std::time::Instant,
}

impl Px4Adapter {
    /// Binds the configured MAVLink receive link and offboard command
    /// uplink. A failed uplink bind degrades to telemetry-only rather
    /// than failing the adapter.
    ///
    /// # Errors
    ///
    /// Returns [`Px4AdapterError::Link`] when the receive link cannot
    /// bind any socket.
    pub async fn start(vehicle: VehicleId, config: Px4Config) -> Result<Self, Px4AdapterError> {
        let link_config = config.link_config();
        let incarnation = pilotage_adapter_api::SourceIncarnation::new(rand_incarnation());
        let mut link = MavlinkLink::start(link_config, incarnation).await?;
        let state = link.state();
        // The gimbal path is wired only when the vehicle is declared to
        // carry one: a bare airframe advertises no `vehicle.gimbal`
        // scope, so a client cannot lease a payload it cannot point. The
        // rate lane is taken exactly once here, so it is always present
        // on this first (and only) take.
        let gimbal = match (config.gimbal, link.take_gimbal_rate_sender()) {
            (true, Some(rates)) => {
                // Acceptance fault injection: suppress the host's gimbal
                // link-loss stop so PX4's own setpoint-timeout is the sole
                // failsafe under test. The typed config permits this ONLY under
                // `Px4Profile::Simulation`, so a real vehicle can never withhold
                // its safe-state command.
                Some(
                    crate::gimbal::Px4GimbalControl::new(
                        link.command_sender(),
                        rates,
                        link_config.system_id,
                        link_config.component_id,
                    )
                    .with_dropped_link_loss_stop(config.drop_gimbal_link_loss_stop()),
                )
            }
            _ => None,
        };
        let uplink = match Px4Uplink::new(config.command_endpoint) {
            Ok(mut uplink) => {
                uplink.set_expected_source(link_config.system_id, link_config.component_id);
                Some(uplink)
            }
            Err(error) => {
                tracing::warn!(%error, "PX4 uplink unavailable; telemetry-only");
                None
            }
        };
        let (frames, camera_bridge, frame_forwarder) =
            camera::spawn_camera_bridge(config.gimbal).await;
        Ok(Self {
            vehicle,
            estimate: Some(EstimateSource {
                state,
                _link: Some(link),
            }),
            uplink,
            gimbal,
            frames,
            _camera_bridge: camera_bridge,
            _frame_forwarder: frame_forwarder,
            arm: None,
            last_seen_heartbeat: None,
            arm_incarnation: pilotage_adapter_api::SourceIncarnation::new(rand_incarnation()),
            gimbal_stamp: sampling::GimbalStamp::new(pilotage_adapter_api::SourceIncarnation::new(
                rand_incarnation(),
            )),
            started_at: std::time::Instant::now(),
            last_reset: None,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        })
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LinkState>>) -> Self {
        Self {
            vehicle,
            estimate: Some(EstimateSource { state, _link: None }),
            uplink: None,
            gimbal: None,
            frames: None,
            _camera_bridge: None,
            _frame_forwarder: None,
            arm: None,
            last_seen_heartbeat: None,
            arm_incarnation: pilotage_adapter_api::SourceIncarnation::new([0; 16]),
            gimbal_stamp: sampling::GimbalStamp::new(pilotage_adapter_api::SourceIncarnation::new(
                [0x60; 16],
            )),
            started_at: std::time::Instant::now(),
            last_reset: None,
            reset_latch: None,
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        }
    }

    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>> {
        self.frames.take()
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: Px4Uplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    /// Installs a test gimbal control path, for tests.
    #[cfg(test)]
    pub(crate) fn with_gimbal(mut self, gimbal: crate::gimbal::Px4GimbalControl) -> Self {
        self.gimbal = Some(gimbal);
        self
    }

    /// Folds the latest heartbeat arm flag into the arm report. Each
    /// newly received heartbeat re-acquires the report (fresh stamp,
    /// advanced sequence), so a silent FC honestly ages into staleness
    /// instead of replaying the last state forever.
    fn observe_arm_report(&mut self) {
        let observed = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
            .and_then(|latest| Some((latest.heartbeat_armed?, latest.last_heartbeat?)));
        let Some((armed, heartbeat_at)) = observed else {
            return;
        };
        if self.last_seen_heartbeat == Some(heartbeat_at) {
            return;
        }
        self.last_seen_heartbeat = Some(heartbeat_at);
        let sequence = self.arm.map_or(0, |report| report.sequence.wrapping_add(1));
        self.arm = Some(ArmReport {
            armed,
            sequence,
            acquired_at: std::time::Instant::now(),
        });
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

/// The video sources this adapter advertises: nothing without a camera
/// bridge, FPV + chase with one, and the gimbal payload feed ONLY when the
/// vehicle is configured with a gimbal — a gimbal-less vehicle must not
/// advertise a source that never paints.
fn advertised_video_sources(bridge_up: bool, gimbal: bool) -> Vec<VideoSource> {
    if !bridge_up {
        return vec![];
    }
    let mut sources = vec![
        VideoSource {
            id: pilotage_adapter_gazebo::FPV_SOURCE_ID.to_owned(),
            description: "onboard forward camera".to_owned(),
        },
        VideoSource {
            id: pilotage_adapter_gazebo::CHASE_SOURCE_ID.to_owned(),
            description: "chase camera".to_owned(),
        },
    ];
    if gimbal {
        sources.push(VideoSource {
            id: pilotage_adapter_gazebo::GIMBAL_SOURCE_ID.to_owned(),
            description: "gimbal payload camera".to_owned(),
        });
    }
    sources
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
        self.advertised_capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        // Per-scope link-loss latch: a frame is suppressed only while ITS scope
        // has a policy engaged, so a gimbal-scope failsafe never suppresses
        // motion frames and vice versa. Applied to both the motion and gimbal
        // routes below.
        if self.link_loss_policy.contains_key(&frame.scope) {
            return rejected_control(tick, RejectReason::LinkLossEngaged);
        }
        if frame.scope.as_str() == GIMBAL_SCOPE {
            return self.apply_gimbal(frame, tick);
        }
        if frame.scope.as_str() == pilotage_adapter_api::SIM_LIFECYCLE_SCOPE {
            return self.apply_sim_lifecycle(frame, tick);
        }
        if let Some(outcome) = self.gated_flight_outcome(frame, tick) {
            return outcome;
        }
        let Some((current_yaw, _pos, _vel)) = self.current_pose() else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };
        let mut action_results = Vec::with_capacity(frame.actions.len());
        for action in &frame.actions {
            match action.kind() {
                ActionKind::Arm => {
                    uplink.begin_arm(current_yaw);
                    action_results.push(ActionResult::accepted(*action));
                }
                // Nothing else is advertised for this scope (sim reset
                // lives on the lifecycle scope), so the session rejects it
                // before delivery — defensive, not a reachable path.
                _ => {
                    action_results.push(ActionResult::rejected(
                        *action,
                        "not supported on the flight scope",
                    ));
                }
            }
        }
        let Some(pilotage_protocol::ControlIntent::Velocity(velocity)) = frame.intent else {
            // An actions-only frame (arm) carries no motion demand; the
            // setpoint stream continues from the next frame.
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results,
            };
        };
        let (sticks, constrained) = control::sticks_from_velocity(&velocity);
        if uplink.send_stick_frame(sticks[0], sticks[1], sticks[2], sticks[3])
            == StickFrameDisposition::UplinkIdle
        {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(RejectReason::UplinkIdle),
                action_results,
            };
        }
        ApplyOutcome {
            tick,
            disposition: if constrained {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
            action_results,
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        self.observe_arm_report();
        let fc_state = sampling::fc_state_sample(
            self.arm,
            self.uplink.as_ref().and_then(Px4Uplink::last_arm_ack),
            self.arm_incarnation,
            self.started_at,
        );
        let gimbal_attitude = self
            .gimbal
            .is_some()
            .then(|| self.gimbal_attitude())
            .flatten();
        let mut batch = self
            .estimate
            .as_ref()
            .map(|source| sampling::mavlink_batch(self.vehicle, &source.state))
            .unwrap_or_default();
        // When no coherent avionics group is available the batch is
        // empty, but FC-state and gimbal-device reports are independent
        // sources that must still reach clients: carry them on a sample
        // even with no pose. Their own stamps drive freshness.
        if batch.samples.is_empty() && (fc_state.is_some() || gimbal_attitude.is_some()) {
            batch.samples.push(TelemetrySample {
                vehicle: self.vehicle,
                tick: self.step(StepBudget { ticks: 0 }).now,
                pose: None,
                speed: None,
                avionics: None,
                sim_truth: None,
                fc_state: None,
                gimbal: None,
            });
        }
        for sample in &mut batch.samples {
            sample.fc_state = fc_state;
            sample.gimbal = gimbal_attitude;
        }
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.maintain();
        }
        if let Some(gimbal) = self.gimbal.as_mut() {
            gimbal.maintain();
        }
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        advertised_video_sources(self._camera_bridge.is_some(), self.gimbal.is_some())
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        self.enact_link_loss_policy(vehicle, scope, policy)
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
