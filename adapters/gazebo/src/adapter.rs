//! `VehicleAdapter` implementation backed by a real Gazebo diff-drive vehicle,
//! driven through the C++ gz-transport sidecar bridge (ADR-0008).
//!
//! Canonical control axes map to the diff-drive command frame: throttle =
//! `LogicalAxisId(2)` -> `linear.x`, yaw = `LogicalAxisId(3)` -> `angular.z`.
//! Cached `BridgeOdometry` becomes a canonical `TelemetrySample`. Raw camera
//! frames are exposed alongside the trait via [`GazeboAdapter::subscribe_frames`]
//! because streaming video does not fit the pull-based `sample_telemetry` shape.

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossPolicy, Pose2d,
    RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch, TelemetrySample,
    VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{LogicalAxisId, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::bridge_client::{BridgeClient, BridgeConfig};
use crate::error::GazeboAdapterError;
use crate::wire::{BridgeControl, BridgeFrame, BridgeOdometry};

/// The control scope this adapter exposes for the diff-drive vehicle.
pub const MOTION_SCOPE: &str = "vehicle.motion";
/// Canonical logical axis carrying throttle (`linear.x`) commands.
pub const THROTTLE_AXIS: u16 = 2;
/// Canonical logical axis carrying yaw (`angular.z`) commands.
pub const YAW_AXIS: u16 = 3;
/// Identifier of the onboard FPV camera video source (source id 0).
pub const FPV_SOURCE_ID: &str = "onboard-fpv";
/// Identifier of the chase camera video source (source id 1).
pub const CHASE_SOURCE_ID: &str = "chase";
/// Wire source id of the onboard FPV camera.
pub const FPV_CAMERA: u8 = 0;
/// Wire source id of the chase camera.
pub const CHASE_CAMERA: u8 = 1;

/// A decoded raw camera frame from the sidecar bridge, paired with the
/// simulation tick it was captured at.
///
/// Exposed via [`GazeboAdapter::subscribe_frames`] alongside the
/// `VehicleAdapter` trait rather than through it: frame delivery is a
/// streaming, backpressure-sensitive concern that does not fit the pull-based
/// `sample_telemetry` shape (ADR-0008).
#[derive(Debug, Clone)]
pub struct RawVideoFrame {
    /// Video source this frame came from: 0 = onboard FPV, 1 = chase. Carried
    /// end to end so the host media pipeline and every reader can route each
    /// frame to the right video source (the wire `source_id` byte).
    pub source_id: u8,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Sidecar-reported pixel format (e.g. `"RGB_INT8"`).
    pub pixel_format: String,
    /// Simulation tick this frame was captured at (sidecar sim time, ns).
    pub tick: SimTick,
    /// Raw pixel bytes, row-major, no padding.
    pub rgb: Vec<u8>,
}

impl From<BridgeFrame> for RawVideoFrame {
    fn from(frame: BridgeFrame) -> Self {
        Self {
            // Bridge `camera_id` is a u32 for wire headroom, but only ids 0
            // (FPV) / 1 (chase) are assigned; an out-of-range id saturates to
            // u8::MAX so the reader routes it to no known source rather than
            // aliasing onto a valid one.
            source_id: u8::try_from(frame.camera_id).unwrap_or(u8::MAX),
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format,
            tick: SimTick::new(frame.sim_time_ns),
            rgb: frame.rgb,
        }
    }
}

/// `VehicleAdapter` implementation that drives a real Gazebo diff-drive
/// vehicle through the gz-transport sidecar bridge.
///
/// The adapter is real-time (ADR-0013): it observes sim time from the bridge's
/// odometry stream and does not itself advance the simulation, so `step` is a
/// no-op reporting the latest observed sim tick.
#[derive(Debug)]
pub struct GazeboAdapter {
    vehicle: VehicleId,
    bridge: BridgeClient,
    frame_rx: Option<mpsc::Receiver<RawVideoFrame>>,
    frame_forwarder: Option<JoinHandle<()>>,
    link_loss_policy: Option<LinkLossPolicy>,
}

impl GazeboAdapter {
    /// Spawns the sidecar bridge, connects to it, and returns a ready adapter
    /// for `vehicle`.
    ///
    /// A background forwarder converts inbound `BridgeFrame`s into
    /// [`RawVideoFrame`]s on a bounded channel drained through
    /// [`Self::subscribe_frames`].
    ///
    /// # Errors
    ///
    /// Returns a [`GazeboAdapterError`] if the listener cannot bind, the bridge
    /// child cannot be spawned, or its inbound connection cannot be accepted.
    pub async fn new(vehicle: VehicleId, config: BridgeConfig) -> Result<Self, GazeboAdapterError> {
        let depth = config.frame_channel_depth.max(1);
        let mut bridge = BridgeClient::spawn_and_connect(config).await?;

        let (raw_tx, raw_rx) = mpsc::channel::<RawVideoFrame>(depth);
        let frame_forwarder = bridge
            .take_frame_rx()
            .map(|bridge_rx| tokio::spawn(forward_frames(bridge_rx, raw_tx)));

        Ok(Self {
            vehicle,
            bridge,
            frame_rx: Some(raw_rx),
            frame_forwarder,
            link_loss_policy: None,
        })
    }

    /// Wires an adapter around a caller-supplied bridge client (no child
    /// process), for in-process fake-bridge tests.
    #[cfg(test)]
    fn from_bridge(vehicle: VehicleId, mut bridge: BridgeClient) -> Self {
        let (raw_tx, raw_rx) = mpsc::channel::<RawVideoFrame>(4);
        let frame_forwarder = bridge
            .take_frame_rx()
            .map(|bridge_rx| tokio::spawn(forward_frames(bridge_rx, raw_tx)));
        Self {
            vehicle,
            bridge,
            frame_rx: Some(raw_rx),
            frame_forwarder,
            link_loss_policy: None,
        }
    }

    /// Takes ownership of the receiver for decoded raw video frames, if not
    /// already taken.
    ///
    /// Frames do not fit the sans-IO `VehicleAdapter` trait's pull-based
    /// telemetry model, so they are exposed through this concrete method for
    /// the host's media pipeline instead (ADR-0008, ADR-0005).
    pub fn subscribe_frames(&mut self) -> Option<mpsc::Receiver<RawVideoFrame>> {
        self.frame_rx.take()
    }

    /// Number of camera frames the bridge reader dropped due to a full frame
    /// channel (a slow media consumer), for diagnostics.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.bridge.dropped_frames()
    }

    /// Reports whether the bridge's background reader is still alive.
    ///
    /// This is exposed alongside the `VehicleAdapter` trait (like
    /// [`Self::subscribe_frames`]) because the trait's pull-based
    /// `sample_telemetry` has no channel for a liveness error. Once this
    /// returns `Err`, cached telemetry is permanently frozen and
    /// [`Self::sample_telemetry`] stops emitting the stale sample, so a host
    /// polling this can neutralize the vehicle instead of trusting dead
    /// odometry (a teleop safety gap).
    ///
    /// # Errors
    ///
    /// Returns [`GazeboAdapterError::ReaderTaskEnded`] once the bridge reader
    /// has exited, carrying the reason.
    pub fn reader_health(&self) -> Result<(), GazeboAdapterError> {
        self.bridge.reader_health()
    }

    fn validate_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != MOTION_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        let known = [
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

    /// Simulation tick derived from the latest observed odometry, or tick 0
    /// before any odometry has arrived.
    fn latest_tick(&self) -> SimTick {
        self.bridge
            .latest_cached()
            .odometry
            .map_or_else(|| SimTick::new(0), |odom| SimTick::new(odom.sim_time_ns))
    }
}

/// Maps canonical axis values in a validated frame onto a diff-drive command.
///
/// Axis values follow the `[-1.0, 1.0]` canonical convention; they are passed
/// through as `linear.x` / `angular.z` in the vehicle's native m/s and rad/s.
/// Absent axes hold neutral (`0.0`) rather than the last value: the host
/// resends the full motion frame each tick under latest-valid-value semantics.
fn control_from_frame(frame: &ScopedControlFrame) -> (BridgeControl, bool) {
    let mut linear_x = 0.0_f64;
    let mut angular_z = 0.0_f64;
    let mut transformed = false;
    for (axis, value) in &frame.payload.axes {
        let (clamped, changed) = clamp_axis(f64::from(*value));
        transformed |= changed;
        if *axis == LogicalAxisId::new(THROTTLE_AXIS) {
            linear_x = clamped;
        } else if *axis == LogicalAxisId::new(YAW_AXIS) {
            angular_z = clamped;
        }
    }
    (
        BridgeControl {
            linear_x,
            angular_z,
        },
        transformed,
    )
}

/// Coerces a raw axis value into `[-1.0, 1.0]`, returning the clamped value and
/// whether it differed from the input. NaN maps to neutral `0.0`; infinities
/// map to the range bounds, so a non-finite value never reaches the bridge.
fn clamp_axis(value: f64) -> (f64, bool) {
    let clamped = if value.is_nan() {
        0.0
    } else {
        value.clamp(-1.0, 1.0)
    };
    (clamped, clamped != value)
}

/// Builds a `TelemetrySample` from a bridge odometry reading.
fn telemetry_from_odometry(vehicle: VehicleId, odom: &BridgeOdometry) -> TelemetrySample {
    TelemetrySample {
        vehicle,
        tick: SimTick::new(odom.sim_time_ns),
        pose: Pose2d {
            x: odom.x,
            y: odom.y,
            heading: odom.heading,
        },
        speed: odom.speed,
    }
}

/// Forwards `BridgeFrame`s to the adapter's `RawVideoFrame` channel until
/// either end closes.
async fn forward_frames(
    mut bridge_rx: mpsc::Receiver<BridgeFrame>,
    raw_tx: mpsc::Sender<RawVideoFrame>,
) {
    while let Some(frame) = bridge_rx.recv().await {
        if raw_tx.send(RawVideoFrame::from(frame)).await.is_err() {
            return;
        }
    }
}

impl Drop for GazeboAdapter {
    fn drop(&mut self) {
        if let Some(handle) = self.frame_forwarder.take() {
            handle.abort();
        }
    }
}

impl VehicleAdapter for GazeboAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: true,
                physically_embodied: false,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: vec![ScopeDescriptor {
                    scope: ScopeId::new(MOTION_SCOPE),
                    axes: vec![
                        LogicalAxisId::new(THROTTLE_AXIS),
                        LogicalAxisId::new(YAW_AXIS),
                    ],
                }],
                link_loss_actions: vec![LinkLossPolicy::Neutralize],
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        if let Err(reason) = self.validate_frame(frame) {
            return ApplyOutcome {
                tick: self.latest_tick(),
                disposition: Disposition::Rejected(reason),
            };
        }
        let (control, transformed) = control_from_frame(frame);
        if !self.bridge.try_send_control(control) {
            return ApplyOutcome {
                tick: self.latest_tick(),
                disposition: Disposition::Rejected(RejectReason::Other(
                    "sidecar bridge control link is closed".to_owned(),
                )),
            };
        }
        ApplyOutcome {
            tick: self.latest_tick(),
            disposition: if transformed {
                Disposition::Transformed
            } else {
                Disposition::Accepted
            },
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        // Once the reader has died the odometry cache is frozen; emitting the
        // last sample would present stale pose/speed as live. The trait has no
        // error channel, so withhold the sample and let a host that cares poll
        // `reader_health` for the reason (a teleop safety gap).
        if self.bridge.reader_health().is_err() {
            return TelemetryBatch::default();
        }
        match self.bridge.latest_cached().odometry {
            Some(odom) => TelemetryBatch {
                samples: vec![telemetry_from_odometry(self.vehicle, &odom)],
            },
            None => TelemetryBatch::default(),
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        vec![
            VideoSource {
                id: FPV_SOURCE_ID.to_owned(),
                description: "onboard forward camera".to_owned(),
            },
            VideoSource {
                id: CHASE_SOURCE_ID.to_owned(),
                description: "chase camera".to_owned(),
            },
        ]
    }

    fn set_link_loss_policy(&mut self, vehicle: VehicleId, policy: Option<LinkLossPolicy>) {
        if vehicle != self.vehicle {
            return;
        }
        self.link_loss_policy = policy;
        // On link loss, halt the vehicle immediately. Only `Neutralize` is
        // advertised (a diff-drive without onboard automation has no richer
        // safe action), so any engaged policy stops the wheels; clearing the
        // policy (`None`, link recovery) leaves the last operator command in
        // effect. A closed control link means the vehicle is already stopping
        // on its own bridge-side timeout, so a failed send is not fatal here.
        if policy.is_some() {
            let stopped = self.bridge.try_send_control(BridgeControl {
                linear_x: 0.0,
                angular_z: 0.0,
            });
            let _ = stopped;
        }
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        // Real-time adapter (ADR-0013): the simulation advances on Gazebo's own
        // clock, observed through the odometry stream. `step` never drives it,
        // so it advances zero ticks and reports the latest observed sim tick.
        StepOutcome {
            advanced: 0,
            now: self.latest_tick(),
        }
    }
}

#[cfg(test)]
mod tests;
