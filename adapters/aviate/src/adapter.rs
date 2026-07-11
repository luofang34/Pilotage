//! `VehicleAdapter` implementation over a selectable Aviate vehicle link
//! (ADR-0019): shared memory when co-located with the SITL, MAVLink over
//! UDP otherwise.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, AvionicsSample, Disposition, ExecutionMode, LinkLossPolicy,
    Pose2d, RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch,
    TelemetrySample, VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{
    ButtonEdge, LogicalAxisId, LogicalButtonId, ScopeId, ScopedControlFrame, VehicleId,
};
use pilotage_timing::SimTick;

use crate::error::AviateAdapterError;
use crate::link::{AviateLink, LatestAviate, LinkConfig};
use crate::shm::{GzStateShm, ShmFreshness};
use crate::uplink::FlightUplink;

mod camera;
mod sampling;
use sampling::{mavlink_batch, yaw_of};

/// The control scope this adapter exposes (issue #12): four canonical
/// flight axes as DJI-style velocity demands.
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
/// Logical button whose press resets the simulation (runs the reset
/// script; SITL-only convenience).
pub const RESET_BUTTON: u16 = 2;
/// Logical button toggling between camera mode (velocity sticks,
/// brake-to-hold) and FPV mode (attitude sticks, direct thrust).
pub const FPV_TOGGLE_BUTTON: u16 = 3;

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever (the same withholding discipline as
/// the Gazebo adapter's dead-reader path).
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Which vehicle link the adapter binds (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AviateLinkMode {
    /// Try the shared-memory block first, fall back to MAVLink/UDP.
    #[default]
    Auto,
    /// Shared-memory state block only (co-located SITL).
    Shm,
    /// MAVLink over UDP only (routed/remote setups, PX4 compatibility).
    Mavlink,
}

#[derive(Debug)]
enum Source {
    Mavlink {
        state: Arc<Mutex<LatestAviate>>,
        // Kept alive for its receive task; dropped with the adapter.
        _link: Option<AviateLink>,
    },
    Shm {
        shm: GzStateShm,
        freshness: ShmFreshness,
        instance: u8,
    },
}

/// Telemetry-only adapter for the Aviate flight controller (ADR-0018).
///
/// Real-time (ADR-0013): the FC/simulation advances on its own clock;
/// `step` reports the latest observed vehicle time as the simulation
/// tick.
#[derive(Debug)]
pub struct AviateAdapter {
    vehicle: VehicleId,
    source: Source,
    uplink: Option<FlightUplink>,
    // Camera path (issue #12): Pilotage's own gz sidecar bridges the
    // flight world's camera topics; absent when the sidecar can't spawn.
    frames: Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
    _camera_bridge: Option<pilotage_adapter_gazebo::BridgeClient>,
    _frame_forwarder: Option<tokio::task::JoinHandle<()>>,
    // Latest armed report from FC heartbeats on the uplink socket.
    armed: Option<bool>,
    last_reset: Option<std::time::Instant>,
    link_loss_policy: Option<LinkLossPolicy>,
    // FPV mode latch (FPV_TOGGLE_BUTTON): attitude sticks + direct
    // thrust instead of velocity sticks + brake-to-hold.
    fpv_mode: bool,
}

impl AviateAdapter {
    /// Binds the vehicle link per `mode` and returns a ready adapter.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when the selected link cannot be
    /// established (`Auto` errors only if both links fail).
    pub async fn start(
        vehicle: VehicleId,
        mode: AviateLinkMode,
        config: LinkConfig,
    ) -> Result<Self, AviateAdapterError> {
        let source = match mode {
            AviateLinkMode::Shm => Self::shm_source(0)?,
            AviateLinkMode::Mavlink => Self::mavlink_source(config).await?,
            AviateLinkMode::Auto => match Self::shm_source(0) {
                Ok(source) => {
                    tracing::info!("Aviate link: shared-memory state block");
                    source
                }
                Err(error) => {
                    tracing::info!(%error, "Aviate shm not available; using MAVLink/UDP");
                    Self::mavlink_source(config).await?
                }
            },
        };
        // A failed uplink bind degrades to telemetry-only rather than
        // failing the adapter: displaying a flight you cannot command
        // beats displaying nothing.
        let uplink = match FlightUplink::new() {
            Ok(uplink) => Some(uplink),
            Err(error) => {
                tracing::warn!(%error, "flight uplink unavailable; telemetry-only");
                None
            }
        };
        let (frames, camera_bridge, frame_forwarder) = camera::spawn_camera_bridge().await;
        Ok(Self {
            vehicle,
            source,
            uplink,
            frames,
            _camera_bridge: camera_bridge,
            _frame_forwarder: frame_forwarder,
            armed: None,
            last_reset: None,
            fpv_mode: false,
            link_loss_policy: None,
        })
    }

    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>> {
        self.frames.take()
    }

    /// Runs the SITL reset script (debounced to one per 5 s): world
    /// reset + FC restart, fire-and-forget. `PILOTAGE_RESET_CMD`
    /// overrides the script path.
    fn spawn_reset(&mut self) {
        let now = std::time::Instant::now();
        if self
            .last_reset
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(5))
        {
            return;
        }
        self.last_reset = Some(now);
        self.armed = None;
        let script = std::env::var("PILOTAGE_RESET_CMD").unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(std::path::Path::parent)
                .map_or_else(|| ".".to_owned(), |p| p.display().to_string())
                + "/scripts/reset-flight-sim.sh"
        });
        tracing::info!(%script, "simulation reset requested from the viewer");
        match std::process::Command::new(&script)
            .arg("aviate_sitl")
            .spawn()
        {
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, %script, "reset script failed to spawn"),
        }
    }

    fn shm_source(instance: u8) -> Result<Source, AviateAdapterError> {
        Ok(Source::Shm {
            shm: GzStateShm::open(instance)?,
            freshness: ShmFreshness::new(),
            instance,
        })
    }

    async fn mavlink_source(config: LinkConfig) -> Result<Source, AviateAdapterError> {
        let link = AviateLink::start(config).await?;
        Ok(Source::Mavlink {
            state: link.state(),
            _link: Some(link),
        })
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LatestAviate>>) -> Self {
        Self {
            vehicle,
            source: Source::Mavlink { state, _link: None },
            uplink: None,
            frames: None,
            _camera_bridge: None,
            _frame_forwarder: None,
            armed: None,
            last_reset: None,
            fpv_mode: false,
            link_loss_policy: None,
        }
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: FlightUplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    /// The vehicle's current measured yaw (radians clockwise from
    /// north) and NED position (zeros before any telemetry).
    fn current_pose(&mut self) -> (f32, [f32; 3]) {
        match &self.source {
            Source::Shm { shm, .. } => shm.read().map_or((0.0, [0.0; 3]), |s| {
                (yaw_of(s.quat_wxyz) as f32, s.pos_ned_m)
            }),
            Source::Mavlink { state, .. } => state.lock().ok().map_or((0.0, [0.0; 3]), |latest| {
                let yaw = latest
                    .attitude
                    .map_or(0.0, |att| yaw_of(att.quat_wxyz) as f32);
                let pos = latest.kinematics.map_or([0.0; 3], |kin| kin.pos_ned_m);
                (yaw, pos)
            }),
        }
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

impl VehicleAdapter for AviateAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: self._camera_bridge.is_some(),
                ..ExecutionMode::default()
            },
            // The flight scope (issue #12): DJI-style velocity control.
            // Without a working uplink the adapter stays telemetry-only
            // (ADR-0018's original shape).
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
        if self.uplink.is_none() {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(RejectReason::UnknownScope),
            };
        }
        if let Err(reason) = self.validate_flight_frame(frame) {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(reason),
            };
        }
        // Reset is scanned before the uplink borrow (it needs &mut self).
        if frame
            .payload
            .edges
            .iter()
            .any(|(b, e)| *e == ButtonEdge::Pressed && *b == LogicalButtonId::new(RESET_BUTTON))
        {
            self.spawn_reset();
        }
        let (current_yaw, current_pos) = self.current_pose();
        let Some(uplink) = self.uplink.as_mut() else {
            // Checked above; unreachable in practice.
            return ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(RejectReason::UnknownScope),
            };
        };

        for (button, edge) in &frame.payload.edges {
            if *edge != ButtonEdge::Pressed {
                continue;
            }
            if *button == LogicalButtonId::new(ARM_BUTTON) {
                uplink.send_arm(true, current_yaw);
            } else if *button == LogicalButtonId::new(DISARM_BUTTON) {
                uplink.send_arm(false, current_yaw);
            } else if *button == LogicalButtonId::new(FPV_TOGGLE_BUTTON) {
                self.fpv_mode = !self.fpv_mode;
                tracing::info!(fpv = self.fpv_mode, "flight mode toggled");
            }
        }

        let mut sticks = [0.0f32; 4];
        let mut transformed = false;
        for (axis, value) in &frame.payload.axes {
            let clamped = if value.is_nan() {
                0.0
            } else {
                value.clamp(-1.0, 1.0)
            };
            transformed |= clamped != *value;
            sticks[usize::from(axis.as_u16().min(3))] = clamped;
        }
        if self.fpv_mode {
            uplink.send_fpv_frame(
                sticks[usize::from(ROLL_AXIS)],
                sticks[usize::from(PITCH_AXIS)],
                sticks[usize::from(THROTTLE_AXIS)],
                sticks[usize::from(YAW_AXIS)],
            );
        } else {
            uplink.send_stick_frame(
                sticks[usize::from(ROLL_AXIS)],
                sticks[usize::from(PITCH_AXIS)],
                sticks[usize::from(THROTTLE_AXIS)],
                sticks[usize::from(YAW_AXIS)],
                current_yaw,
                current_pos,
            );
        }
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
        if let Some(uplink) = self.uplink.as_mut()
            && let Some(armed) = uplink.poll_fc()
        {
            self.armed = Some(armed);
        }
        let arm_state = match self.armed {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        match &mut self.source {
            Source::Mavlink { state, .. } => mavlink_batch(self.vehicle, state, arm_state),
            Source::Shm {
                shm,
                freshness,
                instance,
            } => {
                let frozen = match shm.read() {
                    Some(sample) => freshness.observe(sample.seq) > WITHHOLD_AFTER,
                    None => freshness.observe_absent() > WITHHOLD_AFTER,
                };
                if frozen {
                    // A frozen or vanished block usually means the
                    // simulator restarted and re-created the shm object;
                    // an old mapping keeps pointing at the orphaned one,
                    // so reattach by name.
                    if let Ok(new_shm) = GzStateShm::open(*instance) {
                        *shm = new_shm;
                        *freshness = ShmFreshness::new();
                    }
                    return TelemetryBatch::default();
                }
                let Some(sample) = shm.read() else {
                    return TelemetryBatch::default();
                };
                let heading = yaw_of(sample.quat_wxyz);
                let speed = f64::from(
                    (sample.vel_ned_mps[0] * sample.vel_ned_mps[0]
                        + sample.vel_ned_mps[1] * sample.vel_ned_mps[1])
                        .sqrt(),
                );
                TelemetryBatch {
                    samples: vec![TelemetrySample {
                        vehicle: self.vehicle,
                        tick: SimTick::new(sample.time_us.wrapping_mul(1_000)),
                        pose: Pose2d {
                            x: f64::from(sample.pos_ned_m[0]),
                            y: f64::from(sample.pos_ned_m[1]),
                            heading,
                        },
                        speed,
                        avionics: Some(AvionicsSample {
                            quat_wxyz: sample.quat_wxyz,
                            rates_rps: sample.rates_rps,
                            pos_ned_m: sample.pos_ned_m,
                            vel_ned_mps: sample.vel_ned_mps,
                            // Simulator ground truth (ADR-0019 v0): fully
                            // valid by construction; the FC-estimate block
                            // with real flags is the link RFC's v1.
                            valid_flags: 0b1111,
                            quality: 0,
                            arm_state,
                        }),
                    }],
                }
            }
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        if self._camera_bridge.is_none() {
            return vec![];
        }
        vec![
            VideoSource {
                id: pilotage_adapter_gazebo::FPV_SOURCE_ID.to_owned(),
                description: "onboard forward camera".to_owned(),
            },
            VideoSource {
                id: pilotage_adapter_gazebo::CHASE_SOURCE_ID.to_owned(),
                description: "chase camera".to_owned(),
            },
        ]
    }

    fn set_link_loss_policy(&mut self, vehicle: VehicleId, policy: Option<LinkLossPolicy>) {
        if vehicle != self.vehicle {
            return;
        }
        self.link_loss_policy = policy;
        // Engaging any policy sends a zero-velocity setpoint: the FC's
        // velocity mode brakes to a hover, which is the only safe action
        // a camera drone has (`Neutralize`). Clearing (link recovery)
        // leaves the FC hovering until the operator commands again.
        if policy.is_some()
            && let Some(uplink) = self.uplink.as_mut()
        {
            uplink.send_neutral();
        }
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        let tick = match &self.source {
            Source::Mavlink { state, .. } => state
                .lock()
                .ok()
                .and_then(|latest| latest.kinematics)
                .map_or(0, |kin| u64::from(kin.time_boot_ms).wrapping_mul(1_000_000)),
            Source::Shm { shm, .. } => shm.read().map_or(0, |s| s.time_us.wrapping_mul(1_000)),
        };
        StepOutcome {
            advanced: 0,
            now: SimTick::new(tick),
        }
    }
}

#[cfg(test)]
mod tests;
