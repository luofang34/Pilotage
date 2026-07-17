//! The telemetry observation this probe tracks and the projection from a
//! decoded `wire::TelemetrySample` onto it. The host carries telemetry,
//! `Pong`, and `FrameRejected` on the same datagram channel distinguished by
//! envelope arm; the arm dispatch lives in `receiver::decode_datagram_event`,
//! which calls this once it has matched the `TelemetrySample` arm.
//!
//! Capture retains the source-role lanes (LINK-04): the planar pose for
//! change detection, plus the simulation-truth and FC-state lanes with
//! their provenance — role, identity, epoch, sequence, clock, integrity —
//! so a recorded session preserves who said what, under which role, over
//! which path.

use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;
use serde::Serialize;

/// Provenance retained verbatim from one stamped lane: every stamp field,
/// so a capture answers who said what, under which role and identity,
/// over which path, at what time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapturedProvenance {
    /// Wire `SourceRole` value.
    pub role: i32,
    /// Wire `SourceIntegrity` value.
    pub integrity: i32,
    /// Adapter-local source id (per role).
    pub source_id: u64,
    /// Attachment/boot identity, hex-encoded verbatim (whatever length
    /// arrived — capture records what was said, validators judge it).
    pub source_incarnation: String,
    /// Source boot/attachment generation.
    pub source_epoch: u32,
    /// Wrapping group sequence.
    pub sequence: u32,
    /// Acquisition time in the declared clock domain, nanoseconds.
    pub acquired_at_ns: u64,
    /// Wire `MeasurementClock` value.
    pub clock: i32,
}

/// The simulation-truth lane as captured: NED position, availability,
/// and provenance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapturedTruth {
    /// Position NED, meters.
    pub pos_ned_m: (f32, f32, f32),
    /// Truth-field availability mask.
    pub valid_flags: u32,
    /// The lane's stamp, retained verbatim.
    pub provenance: CapturedProvenance,
}

/// The FC-state lane as captured: arm state and provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapturedFcState {
    /// 0 unknown, 1 disarmed, 2 armed.
    pub arm_state: u32,
    /// The lane's stamp, retained verbatim.
    pub provenance: CapturedProvenance,
}

/// The operational-estimate lane's stamps and authorization as captured;
/// the numeric estimate itself is display material, not capture material.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapturedEstimate {
    /// Attitude group stamp, when that group was published.
    pub attitude_stamp: Option<CapturedProvenance>,
    /// Kinematics group stamp, when that group was published.
    pub kinematics_stamp: Option<CapturedProvenance>,
    /// Estimator-status authorization stamp, when supplied.
    pub estimator_status_stamp: Option<CapturedProvenance>,
    /// Effective latched authorization mask.
    pub valid_flags: u32,
    /// Effective latched quality.
    pub quality: u32,
}

/// One decoded telemetry observation: vehicle identity, the planar pose,
/// and every stamped role lane, with the client-local receive timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryObservation {
    /// Client-local monotonic timestamp at datagram receipt.
    pub received_at: MonoTimestamp,
    /// Vehicle the sample describes, when the wire carried one.
    pub vehicle: Option<u64>,
    /// Reported planar pose, used to detect a change since the last sample.
    pub pose: Option<(f32, f32, f32)>,
    /// Operational-estimate stamps and authorization, when published.
    pub estimate: Option<CapturedEstimate>,
    /// Simulation-truth lane, when published with its provenance stamp.
    pub sim_truth: Option<CapturedTruth>,
    /// FC-state lane, when published with its provenance stamp.
    pub fc_state: Option<CapturedFcState>,
}

fn provenance(stamp: &wire::MeasurementStamp) -> CapturedProvenance {
    CapturedProvenance {
        role: stamp.role,
        integrity: stamp.integrity,
        source_id: stamp.source_id,
        source_incarnation: stamp
            .source_incarnation
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        source_epoch: stamp.source_epoch,
        sequence: stamp.sequence,
        acquired_at_ns: stamp.acquired_at_ns,
        clock: stamp.clock,
    }
}

/// Extracts the fields this probe records from an already-decoded
/// [`wire::TelemetrySample`], stamping it with the client-local
/// `received_at`. Lanes without a provenance stamp are dropped, never
/// defaulted: unattributed data is not evidence.
#[must_use]
pub fn observation_from_sample(
    sample: &wire::TelemetrySample,
    received_at: MonoTimestamp,
) -> TelemetryObservation {
    TelemetryObservation {
        received_at,
        vehicle: sample.vehicle.map(|vehicle| vehicle.value),
        pose: sample
            .pose
            .map(|pose| (pose.x_m, pose.y_m, pose.heading_rad)),
        estimate: sample.avionics.as_ref().map(|avionics| CapturedEstimate {
            attitude_stamp: avionics.attitude_stamp.as_ref().map(provenance),
            kinematics_stamp: avionics.kinematics_stamp.as_ref().map(provenance),
            estimator_status_stamp: avionics.estimator_status_stamp.as_ref().map(provenance),
            valid_flags: avionics.valid_flags,
            quality: avionics.quality,
        }),
        sim_truth: sample.sim_truth.as_ref().and_then(|truth| {
            let stamp = truth.stamp.as_ref()?;
            Some(CapturedTruth {
                pos_ned_m: (truth.pos_n_m, truth.pos_e_m, truth.pos_d_m),
                valid_flags: truth.valid_flags,
                provenance: provenance(stamp),
            })
        }),
        fc_state: sample.fc_state.as_ref().and_then(|state| {
            let stamp = state.stamp.as_ref()?;
            Some(CapturedFcState {
                arm_state: state.arm_state,
                provenance: provenance(stamp),
            })
        }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::observation_from_sample;
    use pilotage_protocol::wire;
    use pilotage_timing::MonoTimestamp;

    fn stamp(role: wire::SourceRole, integrity: wire::SourceIntegrity) -> wire::MeasurementStamp {
        wire::MeasurementStamp {
            role: role as i32,
            integrity: integrity as i32,
            source_id: 0x01be,
            source_epoch: 2,
            sequence: 40,
            acquired_at_ns: 1_000_000,
            clock: wire::MeasurementClock::Simulation as i32,
            source_incarnation: vec![0x11; 16],
        }
    }

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
            avionics: None,
            sim_truth: None,
            fc_state: None,
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(100));
        assert_eq!(observation.pose, Some((1.5, 2.5, 0.25)));
        assert_eq!(observation.received_at, MonoTimestamp::from_nanos(100));
        assert_eq!(observation.sim_truth, None);
        assert_eq!(observation.fc_state, None);
    }

    #[test]
    fn missing_pose_stays_missing() {
        let sample = wire::TelemetrySample {
            vehicle: None,
            tick: None,
            observed_at: None,
            pose: None,
            velocity: None,
            avionics: None,
            sim_truth: None,
            fc_state: None,
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(0));
        assert_eq!(observation.pose, None);
    }

    #[test]
    fn capture_retains_role_lanes_with_full_provenance() {
        let sample = wire::TelemetrySample {
            vehicle: Some(wire::VehicleId { value: 1 }),
            tick: Some(wire::SimTick { value: 1_000_000 }),
            observed_at: Some(wire::MonoTimestamp { nanos: 5 }),
            pose: None,
            velocity: None,
            avionics: None,
            sim_truth: Some(Box::new(wire::SimTruthState {
                quat_w: 1.0,
                pos_n_m: 2.0,
                pos_e_m: 1.0,
                pos_d_m: -3.0,
                valid_flags: 0b1101,
                stamp: Some(stamp(
                    wire::SourceRole::SimulationTruth,
                    wire::SourceIntegrity::Unprotected,
                )),
                ..Default::default()
            })),
            fc_state: Some(Box::new(wire::FcState {
                arm_state: 2,
                stamp: Some(stamp(
                    wire::SourceRole::FcState,
                    wire::SourceIntegrity::ChecksummedOnly,
                )),
            })),
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(9));
        let truth = observation.sim_truth.expect("truth lane captured");
        assert_eq!(truth.pos_ned_m, (2.0, 1.0, -3.0));
        assert_eq!(truth.valid_flags, 0b1101);
        assert_eq!(
            truth.provenance.role,
            wire::SourceRole::SimulationTruth as i32
        );
        assert_eq!(
            truth.provenance.integrity,
            wire::SourceIntegrity::Unprotected as i32
        );
        let fc_state = observation.fc_state.expect("fc lane captured");
        assert_eq!(fc_state.arm_state, 2);
        assert_eq!(fc_state.provenance.role, wire::SourceRole::FcState as i32);
        assert_eq!(fc_state.provenance.source_id, 0x01be);
    }

    #[test]
    fn unstamped_lanes_are_dropped_not_defaulted() {
        let sample = wire::TelemetrySample {
            vehicle: Some(wire::VehicleId { value: 1 }),
            tick: None,
            observed_at: None,
            pose: None,
            velocity: None,
            avionics: None,
            sim_truth: Some(Box::new(wire::SimTruthState {
                pos_n_m: 2.0,
                stamp: None,
                ..Default::default()
            })),
            fc_state: Some(Box::new(wire::FcState {
                arm_state: 2,
                stamp: None,
            })),
        };
        let observation = observation_from_sample(&sample, MonoTimestamp::from_nanos(1));
        assert_eq!(observation.sim_truth, None, "truth without provenance");
        assert_eq!(observation.fc_state, None, "fc state without provenance");
    }
}
