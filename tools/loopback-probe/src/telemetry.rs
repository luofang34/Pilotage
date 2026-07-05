//! The telemetry observation this probe tracks and the projection from a
//! decoded `wire::TelemetrySample` onto it. The host carries telemetry,
//! `Pong`, and `FrameRejected` on the same datagram channel distinguished by
//! envelope arm; the arm dispatch lives in `receiver::decode_datagram_event`,
//! which calls this once it has matched the `TelemetrySample` arm.
//!
//! `pilotage-protocol` has no domain wrapper for `TelemetrySample` yet (only
//! the generated `wire::` type), so this module reads the `wire::` payload
//! directly rather than adding one: this binary only needs the pose to detect
//! "did telemetry change", not a full domain model.

use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;

/// One decoded telemetry observation: the fields this probe needs to detect
/// a pose change, plus the client-local receive timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetryObservation {
    /// Client-local monotonic timestamp at datagram receipt.
    pub received_at: MonoTimestamp,
    /// Reported planar pose, used to detect a change since the last sample.
    pub pose: (f32, f32, f32),
}

/// Extracts the pose fields this probe tracks from an already-decoded
/// [`wire::TelemetrySample`], stamping it with the client-local
/// `received_at`. Called by the receiver's datagram-arm router
/// (`receiver::decode_datagram_event`) once it has matched the
/// `TelemetrySample` arm.
#[must_use]
pub fn observation_from_sample(
    sample: &wire::TelemetrySample,
    received_at: MonoTimestamp,
) -> TelemetryObservation {
    let pose = sample.pose.unwrap_or_default();
    TelemetryObservation {
        received_at,
        pose: (pose.x_m, pose.y_m, pose.heading_rad),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::observation_from_sample;
    use pilotage_protocol::wire;
    use pilotage_timing::MonoTimestamp;

    #[test]
    fn projects_pose_from_sample() {
        let sample = wire::TelemetrySample {
            vehicle: Some(wire::VehicleId { value: 1 }),
            tick: Some(wire::SimTick { value: 0 }),
            observed_at: Some(wire::MonoTimestamp { nanos: 0 }),
            pose: Some(wire::Pose2d {
                x_m: 1.5,
                y_m: 2.5,
                heading_rad: 0.25,
            }),
            velocity: None,
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(100));
        assert_eq!(observation.pose, (1.5, 2.5, 0.25));
        assert_eq!(observation.received_at, MonoTimestamp::from_nanos(100));
    }

    #[test]
    fn missing_pose_projects_to_origin() {
        let sample = wire::TelemetrySample {
            vehicle: None,
            tick: None,
            observed_at: None,
            pose: None,
            velocity: None,
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(0));
        assert_eq!(observation.pose, (0.0, 0.0, 0.0));
    }
}
