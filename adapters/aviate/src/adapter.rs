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
    link_loss_policy: Option<LinkLossPolicy>,
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
        let (frames, camera_bridge, frame_forwarder) = Self::camera_bridge().await;
        Ok(Self {
            vehicle,
            source,
            uplink,
            frames,
            _camera_bridge: camera_bridge,
            _frame_forwarder: frame_forwarder,
            link_loss_policy: None,
        })
    }

    /// Spawns the gz camera sidecar for the flight world's `/camera` and
    /// `/chase_camera` topics, degrading to no-video when it can't
    /// (`PILOTAGE_AVIATE_CAMERA=off` disables the attempt).
    #[allow(clippy::type_complexity)]
    async fn camera_bridge() -> (
        Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
        Option<pilotage_adapter_gazebo::BridgeClient>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        if std::env::var("PILOTAGE_AVIATE_CAMERA").as_deref() == Ok("off") {
            return (None, None, None);
        }
        let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
        let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
        let config = pilotage_adapter_gazebo::BridgeConfig::new("x500", bin);
        match pilotage_adapter_gazebo::BridgeClient::spawn_and_connect(config).await {
            Ok(mut bridge) => {
                let (tx, rx) = tokio::sync::mpsc::channel(4);
                let forwarder = bridge.take_frame_rx().map(|mut bridge_rx| {
                    tokio::spawn(async move {
                        while let Some(frame) = bridge_rx.recv().await {
                            let raw = pilotage_adapter_gazebo::RawVideoFrame::from(frame);
                            if tx.send(raw).await.is_err() {
                                return;
                            }
                        }
                    })
                });
                tracing::info!("Aviate camera sidecar up (FPV + chase)");
                (Some(rx), Some(bridge), forwarder)
            }
            Err(error) => {
                tracing::warn!(%error, "camera sidecar unavailable; no video");
                (None, None, None)
            }
        }
    }

    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>> {
        self.frames.take()
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
            link_loss_policy: None,
        }
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: FlightUplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    /// The vehicle's current measured yaw, radians clockwise from north
    /// (zero before any telemetry).
    fn current_yaw(&mut self) -> f32 {
        match &self.source {
            Source::Shm { shm, .. } => shm.read().map_or(0.0, |s| Self::yaw_of(s.quat_wxyz) as f32),
            Source::Mavlink { state, .. } => state
                .lock()
                .ok()
                .and_then(|latest| latest.attitude)
                .map_or(0.0, |att| Self::yaw_of(att.quat_wxyz) as f32),
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

    /// Yaw extracted from the body→NED quaternion (heading, radians
    /// clockwise from north).
    fn yaw_of(q: [f32; 4]) -> f64 {
        let (w, x, y, z) = (
            f64::from(q[0]),
            f64::from(q[1]),
            f64::from(q[2]),
            f64::from(q[3]),
        );
        (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
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
        let current_yaw = self.current_yaw();
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
        uplink.send_stick_frame(
            sticks[usize::from(ROLL_AXIS)],
            sticks[usize::from(PITCH_AXIS)],
            sticks[usize::from(THROTTLE_AXIS)],
            sticks[usize::from(YAW_AXIS)],
            current_yaw,
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
        match &mut self.source {
            Source::Mavlink { state, .. } => mavlink_batch(self.vehicle, state),
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
                let heading = Self::yaw_of(sample.quat_wxyz);
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

/// The MAVLink-path sampling, unchanged semantics from ADR-0018.
fn mavlink_batch(vehicle: VehicleId, state: &Arc<Mutex<LatestAviate>>) -> TelemetryBatch {
    let Ok(latest) = state.lock() else {
        return TelemetryBatch::default();
    };
    let Some(kin) = latest.kinematics else {
        return TelemetryBatch::default();
    };
    if kin.received_at.elapsed() > WITHHOLD_AFTER {
        return TelemetryBatch::default();
    }
    let attitude = latest
        .attitude
        .filter(|att| att.received_at.elapsed() <= WITHHOLD_AFTER);

    let heading = attitude.map_or(0.0, |att| AviateAdapter::yaw_of(att.quat_wxyz));
    let speed = f64::from(
        (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1]).sqrt(),
    );
    let avionics = attitude.map(|att| AvionicsSample {
        quat_wxyz: att.quat_wxyz,
        rates_rps: att.rates_rps,
        pos_ned_m: kin.pos_ned_m,
        vel_ned_mps: kin.vel_ned_mps,
        // Aviate's wire subset does not carry its StateValidFlags /
        // EstimateQuality yet (ADR-0018 names the gap); freshness is
        // the only validity dimension this link can honestly claim.
        valid_flags: 0b1111,
        quality: 0,
    });
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(u64::from(kin.time_boot_ms).wrapping_mul(1_000_000)),
            pose: Pose2d {
                x: f64::from(kin.pos_ned_m[0]),
                y: f64::from(kin.pos_ned_m[1]),
                heading,
            },
            speed,
            avionics,
        }],
    }
}

#[cfg(test)]
mod tests;
