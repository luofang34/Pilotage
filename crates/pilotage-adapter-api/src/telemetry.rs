//! Telemetry sampling vocabulary (ADR-0008).

use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

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

/// A single vehicle's telemetry at one simulation tick.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySample {
    /// Vehicle this sample describes.
    pub vehicle: VehicleId,
    /// Simulation tick this sample was taken at.
    pub tick: SimTick,
    /// Planar pose at this tick.
    pub pose: Pose2d,
    /// Scalar speed at this tick, in the adapter's native units.
    pub speed: f64,
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
            pose: Pose2d {
                x: 1.0,
                y: 2.0,
                heading: 0.5,
            },
            speed: 3.0,
        };
        assert_eq!(sample.pose.x, 1.0);
        assert_eq!(sample.speed, 3.0);
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
