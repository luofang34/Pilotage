//! Telemetry sampling vocabulary (ADR-0008).

use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

/// The monotonic clock in which a measurement's acquisition timestamp is
/// expressed. Timestamps from different domains are never subtracted without
/// an explicit correlation supplied by the adapter boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementClock {
    /// Monotonic time since the producing vehicle computer booted.
    VehicleBoot,
    /// Monotonic simulation time supplied by the simulator.
    Simulation,
    /// Monotonic time on the ground host that received the observation,
    /// for reports whose wire carries no source timestamp (an FC
    /// heartbeat). Receive time is not acquisition time; consumers may
    /// only reason about staleness in this domain, never correlate it
    /// with vehicle or simulation clocks without an explicit mapping.
    HostMonotonic,
}

/// Opaque identity of one source attachment or boot instance.
///
/// Unlike [`MeasurementStamp::source_epoch`], this value is compared only for
/// equality. A new incarnation cannot be ordered relative to an earlier one;
/// the receiver must authorize that transition at a lifecycle boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceIncarnation([u8; 16]);

impl SourceIncarnation {
    /// Constructs an incarnation from its complete 128-bit representation.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the complete opaque representation.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Explicit role of the source behind a measurement. Role is carried in
/// provenance — never encoded into id ranges — so a configured source id
/// can collide across roles without ambiguity, and consumers gate on the
/// role itself (panels and control accept only
/// [`SourceRole::OperationalEstimate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRole {
    /// FC estimator output: the only role eligible for primary panels
    /// and operational command construction.
    OperationalEstimate,
    /// Simulator ground truth: logging, assertions, and comparison in
    /// simulation profiles only.
    SimulationTruth,
    /// FC-owned vehicle state (arm/mode/failsafe) reports.
    FcState,
    /// Video capture identity for camera frames.
    VideoCapture,
    /// Payload-device orientation/state (gimbal attitude) relayed over
    /// the FC link: never a vehicle estimate, never eligible for control
    /// validation, carrying the device's own boot clock.
    PayloadDevice,
}

/// Integrity classification of the path that delivered an observation.
/// The distinction that matters end-to-end is authenticated source data
/// versus merely checksummed or unprotected observations; a consumer
/// making a safety claim must require the level the claim needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIntegrity {
    /// Cryptographically authenticated end-to-end source data.
    Authenticated,
    /// Checksummed (CRC-style) but unauthenticated transport.
    ChecksummedOnly,
    /// No integrity protection beyond the transport's own boundaries
    /// (a local shared-memory mapping relies on host process isolation).
    Unprotected,
}

/// Identity and acquisition stamp for one independently advancing
/// measurement group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurementStamp {
    /// Explicit source role; consumers gate on this, never on id ranges.
    pub role: SourceRole,
    /// Integrity classification of the path that delivered this
    /// observation; every role carries it so authenticated, checksummed,
    /// and unprotected inputs stay distinguishable end to end.
    pub integrity: SourceIntegrity,
    /// Adapter-defined source identifier, stable within one vehicle and
    /// one role. Ids may collide across roles; the role disambiguates.
    pub source_id: u64,
    /// Opaque attachment/boot identity for the producing source.
    pub source_incarnation: SourceIncarnation,
    /// Source boot/attachment generation. A reset changes this value.
    pub source_epoch: u32,
    /// Wrapping group sequence, advanced only for a new measurement.
    pub sequence: u32,
    /// Acquisition time in nanoseconds in [`Self::clock`].
    pub acquired_at_ns: u64,
    /// Clock domain for [`Self::acquired_at_ns`].
    pub clock: MeasurementClock,
}

/// A planar pose: position and heading, independent of any specific vehicle
/// model's internal representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose2d {
    /// X coordinate in the adapter's world frame.
    pub x: f64,
    /// Y coordinate in the adapter's world frame.
    pub y: f64,
    /// Heading in radians, adapter-defined zero and winding direction.
    pub heading: f64,
}

/// One independently published attitude/rates measurement group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvionicsAttitudeSample {
    /// Attitude quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Body rates (p, q, r) in radians/second.
    pub rates_rps: [f32; 3],
    /// Identity and acquisition time of this group measurement.
    pub stamp: MeasurementStamp,
}

/// One independently published position/velocity measurement group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvionicsKinematicsSample {
    /// Position (north, east, down) in meters from the local origin.
    pub pos_ned_m: [f32; 3],
    /// Velocity (north, east, down) in meters/second.
    pub vel_ned_mps: [f32; 3],
    /// Identity and acquisition time of this group measurement.
    pub stamp: MeasurementStamp,
}

/// Raw avionics state estimate for flight vehicles (ADR-0018): the FC
/// estimator's output, not display-ready numbers and never simulator
/// truth — a simulator oracle publishes [`SimTruthSample`] instead, and
/// the two are not interchangeable. Ground vehicles leave
/// [`TelemetrySample::avionics`] as `None`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvionicsSample {
    /// Attitude/rates group, or `None` when it was not supplied.
    pub attitude: Option<AvionicsAttitudeSample>,
    /// Position/velocity group, or `None` when it was not supplied.
    pub kinematics: Option<AvionicsKinematicsSample>,
    /// Identity and acquisition time of the estimator status observation
    /// backing the effective authorization, or `None` when no explicit
    /// authorization was supplied.
    pub estimator_status_stamp: Option<MeasurementStamp>,
    /// Effective latched authorization bitmask: bit0 attitude, bit1 rates,
    /// bit2 position, bit3 velocity. This can include fail-closed downgrades
    /// relative to the raw status observation and is meaningful only when
    /// [`Self::estimator_status_stamp`] is present.
    pub valid_flags: u32,
    /// Effective latched estimate quality: 0 good, 1 degraded, 2 unusable.
    /// This can include fail-closed downgrades relative to the raw status
    /// observation and is meaningful only when
    /// [`Self::estimator_status_stamp`] is present.
    pub quality: u32,
}

/// One coherent simulator ground-truth sample: a simulation oracle for
/// logging, test assertions, and estimate-versus-truth comparison in
/// simulation profiles only. It is a distinct type from
/// [`AvionicsSample`] so truth can never be passed where an FC
/// operational estimate is required: it drives no primary panel and no
/// operational command construction, and it is not a fallback for a
/// missing estimate. Physical profiles must not synthesize one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimTruthSample {
    /// Attitude quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Position NED, meters.
    pub pos_ned_m: [f32; 3],
    /// Velocity NED, m/s.
    pub vel_ned_mps: [f32; 3],
    /// Which truth fields this sample carries, in the same bit positions
    /// as the estimate's authorization mask: bit0 attitude, bit1 rates,
    /// bit2 position, bit3 velocity. Availability only — truth has no
    /// estimator authorization to claim.
    pub valid_flags: u32,
    /// Identity, acquisition time, and integrity of this truth
    /// observation.
    pub stamp: MeasurementStamp,
}

/// The FC's acknowledgement of the most recent commanded arm or disarm
/// (COMMAND_ACK) — enactment truth for the operator. It rides FC-state
/// provenance, so the verdict ages with the report that carried it; the
/// action-result path's command-acceptance semantics are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FcCommandAck {
    /// True when the acknowledged command was an arm, false for a disarm.
    pub arm: bool,
    /// The raw MAV_RESULT the FC returned (0 = accepted).
    pub result: u32,
}

/// FC-owned vehicle state (arm today; mode/failsafe belong here as they
/// arrive) with its own provenance: the FC is the only author, and the
/// stamp records which link observation reported it — it is never merged
/// unstamped into an estimate or truth sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FcStateSample {
    /// Arm state as the FC reports it: 0 unknown, 1 disarmed, 2 armed.
    pub arm_state: u32,
    /// The FC's answer to the most recent commanded arm/disarm, when one
    /// has been observed. A refusal here is the only signal that turns
    /// "the command was taken" into "the FC did not do it".
    pub last_command: Option<FcCommandAck>,
    /// Identity and acquisition time of the FC report carrying this state.
    pub stamp: MeasurementStamp,
}

/// Gimbal payload-device orientation (Gimbal Protocol v2 attitude
/// status) with its own provenance: device state relayed over the FC
/// link, never a vehicle estimate and never an input to control
/// validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalAttitudeSample {
    /// Orientation quaternion (w, x, y, z); vehicle-frame yaw unless
    /// the device declares an earth-frame yaw mode.
    pub quat_wxyz: [f32; 4],
    /// Device angular velocity (rad/s); NaN encodes device-unknown.
    pub rates_rps: [f32; 3],
    /// GIMBAL_DEVICE_FLAGS in effect (mode/lock bits).
    pub flags: u32,
    /// Non-zero reports a device failure condition; carried so a
    /// consumer can surface a degraded payload without re-deriving it.
    pub failure_flags: u32,
    /// Identity and acquisition time of the device report.
    pub stamp: MeasurementStamp,
}

/// A single vehicle's telemetry at one simulation tick.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySample {
    /// Vehicle this sample describes.
    pub vehicle: VehicleId,
    /// Simulation tick this sample was taken at.
    pub tick: SimTick,
    /// Planar pose at this tick, or `None` when its source groups are absent.
    pub pose: Option<Pose2d>,
    /// Scalar speed at this tick, or `None` when it is not measured.
    pub speed: Option<f64>,
    /// Raw FC avionics estimate for flight vehicles; `None` for ground
    /// vehicles (ADR-0018) and whenever no operational estimate exists —
    /// simulator truth is never projected here.
    pub avionics: Option<AvionicsSample>,
    /// Simulator ground-truth oracle, present only in simulation
    /// profiles. Independent of [`Self::avionics`] in identity, epoch,
    /// sequence, clock, and validity; not eligible as an operational
    /// fallback.
    pub sim_truth: Option<SimTruthSample>,
    /// FC-owned arm/mode state with its own provenance stamp.
    pub fc_state: Option<FcStateSample>,
    /// Gimbal payload-device orientation with its own provenance stamp.
    pub gimbal: Option<GimbalAttitudeSample>,
}

/// A batch of telemetry samples returned from a single `sample_telemetry`
/// call, potentially covering multiple vehicles or ticks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TelemetryBatch {
    /// Samples in this batch.
    pub samples: Vec<TelemetrySample>,
}

/// A video or camera source a vehicle exposes (ADR-0008); adapters that are
/// not `render_capable` report an empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSource {
    /// Identifier for this video source, unique within the adapter.
    pub id: String,
    /// Human-readable description (e.g. `"forward camera"`).
    pub description: String,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Pose2d, TelemetryBatch, TelemetrySample, VideoSource};
    use pilotage_protocol::VehicleId;
    use pilotage_timing::SimTick;

    #[test]
    fn telemetry_batch_default_is_empty() {
        let batch = TelemetryBatch::default();
        assert!(batch.samples.is_empty());
    }

    #[test]
    fn telemetry_sample_holds_pose_and_speed() {
        let sample = TelemetrySample {
            vehicle: VehicleId::new(1),
            tick: SimTick::new(2),
            pose: Some(Pose2d {
                x: 1.0,
                y: 2.0,
                heading: 0.5,
            }),
            speed: Some(3.0),
            avionics: None,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
        };
        assert_eq!(sample.pose.expect("pose").x, 1.0);
        assert_eq!(sample.speed, Some(3.0));
    }

    #[test]
    fn video_source_holds_id_and_description() {
        let source = VideoSource {
            id: "cam0".to_owned(),
            description: "forward camera".to_owned(),
        };
        assert_eq!(source.id, "cam0");
    }
}
