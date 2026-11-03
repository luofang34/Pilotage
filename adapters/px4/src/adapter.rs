//! The PX4 `VehicleAdapter`: telemetry sampling from the shared
//! MAVLink link and offboard flight control with the same gate
//! discipline as the Aviate adapter (link-loss latch followed by the
//! commanded-reset latch).

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, LinkLossEnactError, LinkLossPolicy, RejectReason,
    StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter, VideoSource,
};
use pilotage_protocol::{VehicleClass, VehicleId};
use std::collections::BTreeMap;

use pilotage_protocol::{LogicalAxisId, ScopeId, ScopedControlFrame};
use pilotage_timing::SimTick;

use pilotage_mavlink::sampling::{
    ArmTracker, VehicleReports, battery_sample, estimate_batch, fc_state_sample,
    landed_state_sample, report_only_sample,
};
use pilotage_mavlink::{CameraEndpoints, LinkState, MavlinkLink, camera_endpoint, gimbal_endpoint};

use crate::config::{Px4Config, Px4Profile};
use crate::error::Px4AdapterError;
use crate::uplink::Px4Uplink;

mod advertisement;
#[cfg(feature = "sim")]
mod camera;
mod camera_link;
#[cfg(test)]
mod camera_tests;
use camera_link::CameraBridge;
#[cfg(not(feature = "sim"))]
use camera_link::no_camera;

mod control;
mod flight;
#[cfg(test)]
mod gimbal_link_loss_tests;
#[cfg(test)]
mod gimbal_tests;
mod link_loss;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use control::rejected_control;

/// The control scope: four canonical flight axes as velocity demands.
pub const FLIGHT_SCOPE: &str = pilotage_protocol::MOTION_SCOPE;
/// The direct-flight scope: body-rate or attitude demands with collective
/// thrust. It shares the flight scope's authority group, so only one of the
/// two is held at a time. Multicopter only.
pub const DIRECT_SCOPE: &str = pilotage_protocol::DIRECT_SCOPE;
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
pub const GIMBAL_SCOPE: &str = gimbal_endpoint::SCOPE;
/// Gimbal-scope button whose press recenters the gimbal.
pub const GIMBAL_NEUTRAL_BUTTON: u16 = gimbal_endpoint::NEUTRAL_BUTTON;

/// Telemetry-only-until-armed adapter for PX4 (ADR-0018). Real-time
/// (ADR-0013): PX4 advances on its own clock; `step` reports the
/// latest observed source time as the simulation tick.
#[derive(Debug)]
pub struct Px4Adapter {
    vehicle: VehicleId,
    vehicle_class: VehicleClass,
    // The shared receive-link cache. PX4's standard ESTIMATOR_STATUS
    // is the authorization source (LINK-04: the estimate is the only
    // basis for state-dependent control; there is no truth oracle).
    estimate: Option<EstimateSource>,
    uplink: Option<Px4Uplink>,
    // The fixed-wing uplink, in place of `uplink` when the airframe is a
    // fixed wing; at most one of the two is present.
    fixed_wing: Option<crate::fixed_wing::FixedWingUplink>,
    // Gimbal-manager command path; rides the receive link's socket
    // because the FC's GCS instance retargets its telemetry stream to
    // the last peer that spoke.
    gimbal: Option<pilotage_mavlink::ManagerControl>,
    camera_control: Option<CameraEndpoints>,
    // Pilotage's gz sidecar bridges the flight-deck rig's camera topics;
    // the adapter remains usable without video when it cannot spawn.
    frames: Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    _camera_bridge: Option<CameraBridge>,
    _frame_forwarder: Option<tokio::task::JoinHandle<()>>,
    // Latest heartbeat-reported arm state; re-acquired per heartbeat so
    // its freshness honestly tracks the FC's liveness.
    arm: ArmTracker,
    arm_incarnation: pilotage_adapter_api::SourceIncarnation,
    // Payload-device stamp discipline for the gimbal lane: a stable
    // gimbal identity plus a reboot-aware epoch, kept apart from the FC
    // arm incarnation so a device reboot never regresses time under a
    // fixed identity+epoch.
    gimbal_stamp: gimbal_endpoint::GimbalStamp,
    started_at: std::time::Instant,
    // Whether the simulator's truth report may reach telemetry. A physical
    // profile must not publish one, whatever arrives on its link.
    publishes_sim_truth: bool,
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
                    pilotage_mavlink::ManagerControl::new(
                        link.command_sender(),
                        rates,
                        link_config.system_id,
                        None,
                    )
                    .map_err(pilotage_mavlink::LinkError::from)?
                    .with_manager_state(link.state())
                    .with_dropped_link_loss_stop(config.drop_gimbal_link_loss_stop()),
                )
            }
            _ => None,
        };
        let started_at = std::time::Instant::now();
        let camera_control = Some(CameraEndpoints::new(
            vehicle,
            state.clone(),
            link.command_sender(),
            link_config.system_id,
            started_at,
        ));
        let (uplink, fixed_wing) = flight::flight_uplinks(&config, &link_config);
        #[cfg(feature = "sim")]
        let (frames, camera_bridge, frame_forwarder) =
            camera::spawn_camera_bridge(config.gimbal).await;
        #[cfg(not(feature = "sim"))]
        let (frames, camera_bridge, frame_forwarder) = no_camera();
        Ok(Self {
            vehicle,
            vehicle_class: config.airframe.vehicle_class(),
            estimate: Some(EstimateSource {
                state,
                _link: Some(link),
            }),
            uplink,
            fixed_wing,
            gimbal,
            camera_control,
            frames,
            _camera_bridge: camera_bridge,
            _frame_forwarder: frame_forwarder,
            arm: ArmTracker::default(),
            arm_incarnation: pilotage_adapter_api::SourceIncarnation::new(rand_incarnation()),
            gimbal_stamp: gimbal_endpoint::GimbalStamp::new(
                pilotage_adapter_api::SourceIncarnation::new(rand_incarnation()),
            ),
            started_at,
            publishes_sim_truth: config.profile() == Px4Profile::Simulation,
            last_reset: None,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        })
    }

    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>> {
        self.frames.take()
    }

    /// Folds the latest heartbeat into the arm report. Each newly received
    /// heartbeat re-acquires the report (fresh stamp, advanced sequence), so
    /// a silent FC honestly ages into staleness instead of replaying the
    /// last state forever.
    fn observe_arm_report(&mut self) {
        if let Some(latest) = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
        {
            self.arm.observe(&latest);
        }
    }

    /// The current operational pose from the FC estimate only.
    fn current_pose(&self) -> Option<pilotage_mavlink::sampling::OperationalPose> {
        let latest = self.estimate.as_ref()?.state.lock().ok()?;
        pilotage_mavlink::sampling::operational_pose(&latest)
    }

    fn validate_flight_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        // A fixed wing advertises no direct scope (see `advertisement.rs`).
        let direct = frame.scope.as_str() == DIRECT_SCOPE && self.uplink.is_some();
        if frame.scope.as_str() != FLIGHT_SCOPE && !direct {
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
            id: pilotage_adapter_api::FPV_SOURCE_ID.to_owned(),
            description: "onboard forward camera".to_owned(),
        },
        VideoSource {
            id: pilotage_adapter_api::CHASE_SOURCE_ID.to_owned(),
            description: "chase camera".to_owned(),
        },
    ];
    if gimbal {
        sources.push(VideoSource {
            id: pilotage_adapter_api::GIMBAL_SOURCE_ID.to_owned(),
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
    fn invalidate_pending_commands(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        if vehicle != self.vehicle {
            return;
        }
        if scope.as_str() == GIMBAL_SCOPE
            && let Some(gimbal) = self.gimbal.as_mut()
        {
            gimbal.invalidate_pending();
        }
        if let Some(camera) = self.camera_control.as_mut() {
            camera.invalidate_pending_commands(vehicle, scope);
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        self.advertised_capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        // Per-scope link-loss latch: a frame is suppressed only while ITS scope
        // (for the direct scope, its flight group) has a policy engaged, so a
        // gimbal-scope failsafe never suppresses motion frames and vice versa.
        if pilotage_adapter_api::link_loss_latch_engaged(&self.link_loss_policy, &frame.scope) {
            return rejected_control(tick, RejectReason::LinkLossEngaged);
        }
        if frame
            .scope
            .as_str()
            .starts_with(camera_endpoint::SCOPE_PREFIX)
        {
            return self.camera_control.as_mut().map_or_else(
                || rejected_control(tick, RejectReason::UnknownScope),
                |camera| camera.apply_control(frame, tick),
            );
        }
        if frame.scope.as_str() == GIMBAL_SCOPE {
            return self.gimbal.as_mut().map_or_else(
                || rejected_control(tick, RejectReason::UnknownScope),
                |gimbal| gimbal_endpoint::apply_control(gimbal, self.vehicle, frame, tick),
            );
        }
        if frame.scope.as_str() == pilotage_adapter_api::SIM_LIFECYCLE_SCOPE {
            return self.apply_sim_lifecycle(frame, tick);
        }
        if let Some(outcome) = self.gated_flight_outcome(frame, tick) {
            return outcome;
        }
        if self.fixed_wing.is_some() {
            return self.apply_fixed_wing_flight(frame, tick);
        }
        self.apply_multicopter_flight(frame, tick)
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        if let Some(gimbal) = self.gimbal.as_mut() {
            gimbal.refresh_discovery();
        }
        self.observe_arm_report();
        let fc_state = fc_state_sample(
            self.arm.report(),
            self.flight_arm_ack(),
            self.estimate.as_ref().and_then(|source| {
                let latest = source.state.lock().ok()?;
                landed_state_sample(&latest, std::time::Instant::now())
            }),
        );
        let gimbal = self
            .gimbal
            .as_ref()
            .map_or_else(gimbal_endpoint::GimbalObservation::default, |gimbal| {
                gimbal_endpoint::sample(gimbal, &self.gimbal_stamp)
            });
        let battery = battery_sample(
            self.estimate
                .as_ref()
                .and_then(|source| source.state.lock().ok()?.battery),
            self.arm_incarnation,
            self.started_at,
        );
        let mut batch = self
            .estimate
            .as_ref()
            .map(|source| estimate_batch(self.vehicle, &source.state))
            .unwrap_or_default();
        let payload = gimbal.payload;
        let reports = VehicleReports {
            fc_state,
            gimbal: gimbal.attitude,
            battery,
        };
        // FC-state, gimbal-device, and battery reports are independent
        // sources that must reach clients with no coherent avionics group:
        // carry them on a sample even with no pose. Their own stamps drive
        // freshness.
        if batch.samples.is_empty() && reports.any() {
            let tick = self.step(StepBudget { ticks: 0 }).now;
            batch.samples.push(report_only_sample(self.vehicle, tick));
        }
        reports.attach(&mut batch);
        self.attach_sim_truth(&mut batch);
        if let Some(payload) = payload {
            if batch.samples.is_empty() {
                batch
                    .samples
                    .push(report_only_sample(self.vehicle, SimTick::new(0)));
            }
            if let Some(sample) = batch.samples.first_mut() {
                sample.payload = Some(payload);
            }
        }
        if let Some(camera) = self.camera_control.as_mut() {
            camera.sample_telemetry(&mut batch);
        }
        self.maintain_flight_uplink();
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
