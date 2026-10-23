//! The `VehicleAdapter` implementation of the simulated multicopter.

mod capability;
mod control;
mod telemetry;
#[cfg(test)]
mod tests;

use std::hash::{BuildHasher, Hasher};

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, LinkLossEnactError, LinkLossPolicy, RawVideoFrame,
    SourceIncarnation, StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter, VideoSource,
};
use pilotage_geo::LocalOriginId;
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use tokio::sync::mpsc;

use crate::battery::Battery;
use crate::camera::CameraWorker;
use crate::camera::render::CameraPose;
use crate::config::SimConfig;
use crate::geodesy::LocalFrame;
use crate::gimbal::Gimbal;
use crate::kinematics::{BodyVelocity, Kinematics};
use crate::lifecycle::FlightPhase;

/// The flight scope, as on the DJI adapter.
pub const MOTION_SCOPE: &str = pilotage_protocol::MOTION_SCOPE;
/// The gimbal scope, as on the DJI adapter.
pub const GIMBAL_SCOPE: &str = "vehicle.gimbal";
/// Simulation time of one step tick, nanoseconds. The session host steps
/// its adapter every 10 ms.
pub const TICK_NS: u64 = 10_000_000;
/// Simulation time of one step tick, seconds.
const TICK_S: f64 = 0.01;
/// A stick command older than this stops: the vehicle hovers, as a DJI
/// virtual stick does when its commands stop.
const COMMAND_TIMEOUT_TICKS: u64 = 50;
/// A gimbal rate older than this stops.
const GIMBAL_TIMEOUT_TICKS: u64 = 50;

/// A deterministic simulated multicopter on the DJI Mini 3 control path.
///
/// It advertises the same scopes, intents, and actions as the DJI adapter
/// with virtual stick, plus Arm and Disarm. Its execution mode states
/// `physically_embodied: false`, its display name starts with `SIM`, and it
/// publishes a simulation-truth group. Each `step` tick advances 10 ms of
/// simulation time.
pub struct SimMulticopterAdapter {
    vehicle: VehicleId,
    config: SimConfig,
    frame: LocalFrame,
    incarnation: SourceIncarnation,
    origin: LocalOriginId,
    tick: u64,
    body: Kinematics,
    phase: FlightPhase,
    gimbal: Gimbal,
    gimbal_rate_at: Option<u64>,
    battery: Battery,
    motion: Option<(BodyVelocity, u64)>,
    motion_policy: Option<LinkLossPolicy>,
    gimbal_policy: Option<LinkLossPolicy>,
    sequences: telemetry::Sequences,
    camera: Option<CameraWorker>,
    frames_taken: bool,
}

impl SimMulticopterAdapter {
    /// A disarmed vehicle on the ground at the configured launch point.
    #[must_use]
    pub fn new(vehicle: VehicleId, config: SimConfig) -> Self {
        let incarnation = fresh_incarnation(config.seed);
        let bytes = incarnation.into_bytes();
        let mut head = [0u8; 8];
        head.copy_from_slice(&bytes[..8]);
        Self {
            vehicle,
            frame: LocalFrame::anchored_at(config.launch.lat_deg, config.launch.lon_deg),
            incarnation,
            // The low bit keeps the height origin declared.
            origin: LocalOriginId(u64::from_le_bytes(head) | 1),
            tick: 0,
            body: Kinematics::at_rest(config.initial_heading_deg.to_radians()),
            phase: FlightPhase::Disarmed,
            gimbal: Gimbal::at(config.initial_gimbal_pitch_deg.to_radians()),
            gimbal_rate_at: None,
            battery: Battery::full(config.battery),
            motion: None,
            motion_policy: None,
            gimbal_policy: None,
            sequences: telemetry::Sequences::default(),
            camera: None,
            frames_taken: false,
            config,
        }
    }

    /// Starts the camera worker and returns its frames, once. Later calls
    /// and a failed start return `None`.
    pub fn subscribe_frames(&mut self) -> Option<mpsc::Receiver<RawVideoFrame>> {
        if self.frames_taken {
            return None;
        }
        self.frames_taken = true;
        match CameraWorker::spawn(
            self.config.camera.clone(),
            self.frame,
            self.config.seed,
            self.incarnation,
        ) {
            Ok((worker, frames)) => {
                self.camera = Some(worker);
                Some(frames)
            }
            Err(error) => {
                tracing::error!(%error, "simulated camera worker did not start");
                None
            }
        }
    }

    /// The flight lifecycle phase.
    #[must_use]
    pub fn phase(&self) -> FlightPhase {
        self.phase
    }

    /// The simulated state, for tests and diagnostics.
    #[must_use]
    pub fn kinematics(&self) -> &Kinematics {
        &self.body
    }

    /// The gimbal pitch, radians.
    #[must_use]
    pub fn gimbal_pitch_rad(&self) -> f64 {
        self.gimbal.pitch_rad()
    }

    /// The remaining battery fraction, 0 to 1.
    #[must_use]
    pub fn battery_remaining(&self) -> f64 {
        self.battery.remaining_fraction()
    }

    /// The camera pose of the present tick.
    #[must_use]
    pub fn camera_pose(&self) -> CameraPose {
        let [lat_deg, lon_deg] = self
            .frame
            .lat_lon(self.body.pos_ned_m[0], self.body.pos_ned_m[1]);
        CameraPose {
            time_ns: self.tick.wrapping_mul(TICK_NS),
            pos_ned_m: self.body.pos_ned_m,
            yaw_rad: self.body.yaw_rad,
            pitch_rad: self.gimbal.pitch_rad(),
            lat_deg,
            lon_deg,
        }
    }

    fn policy(&self, scope: &str) -> Option<LinkLossPolicy> {
        match scope {
            MOTION_SCOPE => self.motion_policy,
            GIMBAL_SCOPE => self.gimbal_policy,
            _ => None,
        }
    }

    fn current_command(&self) -> BodyVelocity {
        if self.motion_policy.is_some() {
            return BodyVelocity::default();
        }
        match self.motion {
            Some((command, at)) if self.tick.saturating_sub(at) <= COMMAND_TIMEOUT_TICKS => command,
            _ => BodyVelocity::default(),
        }
    }

    fn step_once(&mut self) {
        if self.battery.is_empty()
            && matches!(self.phase, FlightPhase::Airborne | FlightPhase::TakingOff)
        {
            tracing::warn!("simulated battery empty; the vehicle lands");
            self.phase.land().ok();
        }
        let command = self.current_command();
        let before = self.phase;
        self.phase
            .step(&mut self.body, command, &self.config.limits, TICK_S);
        if self.phase != before {
            tracing::info!(
                from = before.mode_name(),
                to = self.phase.mode_name(),
                height_m = self.body.height_m(),
                "simulated multicopter lifecycle"
            );
        }
        let gimbal_stale = self
            .gimbal_rate_at
            .is_none_or(|at| self.tick.saturating_sub(at) > GIMBAL_TIMEOUT_TICKS);
        if self.gimbal_policy.is_some() || gimbal_stale {
            self.gimbal.stop();
        }
        self.gimbal.step(TICK_S);
        self.battery
            .step(self.phase.draw(), self.body.ground_speed_mps(), TICK_S);
        self.tick = self.tick.wrapping_add(1);
        self.submit_camera_pose();
    }

    fn submit_camera_pose(&self) {
        let Some(camera) = self.camera.as_ref() else {
            return;
        };
        let fps = u64::from(self.config.camera.fps.clamp(1, 100));
        let every = (100 / fps).max(1);
        if self.tick.is_multiple_of(every) {
            camera.submit(self.camera_pose());
        }
    }
}

/// A fresh attachment identity for each adapter. The seed keeps the
/// trajectory deterministic; the identity must differ across restarts so
/// a reader sees a new source.
fn fresh_incarnation(seed: u64) -> SourceIncarnation {
    let state = std::collections::hash_map::RandomState::new();
    let mut bytes = [0u8; 16];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_u64(seed);
        hasher.write_usize(index);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    SourceIncarnation::new(bytes)
}

impl VehicleAdapter for SimMulticopterAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        capability::capabilities(
            self.vehicle,
            &self.config.display_name(),
            &self.config.limits,
            self.camera.is_some(),
        )
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        self.apply(frame)
    }

    fn invalidate_pending_commands(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        if vehicle != self.vehicle {
            return;
        }
        match scope.as_str() {
            MOTION_SCOPE => self.motion = None,
            GIMBAL_SCOPE => self.gimbal.stop(),
            _ => {}
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        TelemetryBatch {
            samples: self.sample().into_iter().collect(),
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        if self.camera.is_none() {
            return Vec::new();
        }
        vec![VideoSource {
            id: pilotage_adapter_api::GIMBAL_SOURCE_ID.to_owned(),
            description: "simulated gimbal camera (aerial imagery)".to_owned(),
        }]
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        // Every engaged policy drives the scope to its safe state: zero
        // velocity is a hover, zero rate holds the gimbal pitch.
        match scope.as_str() {
            MOTION_SCOPE => {
                self.motion_policy = policy;
                self.motion = None;
            }
            GIMBAL_SCOPE => {
                self.gimbal_policy = policy;
                self.gimbal.stop();
            }
            _ => {}
        }
        Ok(())
    }

    fn step(&mut self, budget: StepBudget) -> StepOutcome {
        for _ in 0..budget.ticks {
            self.step_once();
        }
        StepOutcome {
            advanced: budget.ticks,
            now: SimTick::new(self.tick),
        }
    }
}
