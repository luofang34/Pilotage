//! Telemetry sampling vocabulary (ADR-0008).

use pilotage_geo::{GeodeticPosition, PositionQuality};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

// Source identity moved to `pilotage-ingress` so the ingress rules can be
// applied where no wire exists (ADR-0018, ADR-0037's local-source path).
// Re-exported here because this crate is the adapter-facing vocabulary.
pub use pilotage_ingress::{
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};

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

/// One geodetic fix: where the vehicle is on the Earth, with the datum the
/// position is measured against fully declared (ADR-0022).
///
/// Absence of the whole sample means no fix. A producer with no fix leaves
/// it out; it never reports a zero, a last-known value, or a position
/// derived from a declared origin — an origin-derived position is a
/// different thing and is not interchangeable with a measured one.
///
/// The position is a [`GeodeticPosition`], so a datum a reader cannot
/// interpret cannot be built: an MSL height needs a declared geoid, an AGL
/// height a declared terrain reference, and an unknown datum is refused
/// rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeodeticFixSample {
    /// Where the vehicle is, and what the position is measured against.
    pub position: GeodeticPosition,
    /// How well the position is known. A reader derives health from this;
    /// the producer states no availability of its own.
    pub quality: PositionQuality,
    /// Identity and acquisition time of this fix.
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
    /// Barometric group, or `None` when it was not supplied.
    pub baro: Option<AvionicsBaroSample>,
    /// The estimator's own geodetic fix, or `None` when the estimator
    /// supplied none. It advances independently of the kinematics group
    /// and carries its own stamp. A simulator's truth oracle publishes a
    /// fix on [`SimTruthSample`] instead; the two are never merged.
    pub geodetic: Option<GeodeticFixSample>,
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

/// The barometric group of one avionics publication: pressure altitude
/// against the ISA standard datum. It approximates true altitude only
/// after a local pressure correction, and the display must label which
/// it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvionicsBaroSample {
    /// Pressure altitude, meters, standard datum (1013.25 hPa).
    pub pressure_alt_m: f32,
    /// Identity and acquisition stamp for this group update.
    pub stamp: MeasurementStamp,
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
    /// The oracle's geodetic position, or `None` when the simulator
    /// declared none. It comes from the same observation as the NED group
    /// above and rides this sample's `stamp`, so it never claims an
    /// advance of its own.
    pub geodetic: Option<GeodeticFixSample>,
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
