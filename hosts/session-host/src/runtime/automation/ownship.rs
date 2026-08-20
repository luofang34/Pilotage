//! Telemetry-to-ownship conversion for the mission principal.
//!
//! Acquisition time on the mission clock is the HOST's publication stamp
//! (the shared monotonic origin): a measurement group's own stamp rides a
//! foreign clock domain (vehicle boot / simulation) and is never
//! subtracted against host time without an explicit correlation
//! (ADR-0009). Source ROLES, by contrast, are taken from the stamps
//! verbatim — the mission engine's honesty gate (ADR-0024) decides what
//! each role may become.

use navigate_contract::MonotonicNanos;
use pilotage_mission::{OwnshipSample, TruthRole};
use pilotage_protocol::wire;

/// Converts one wire telemetry sample into an ownship sample, preferring
/// the simulation-truth group, then stamped avionics kinematics, then —
/// only when the host declares its planar poses ARE simulator state
/// (`planar_pose_is_truth`) — the planar pose. `received_at` is the
/// fallback acquisition time when the sample carries no host publication
/// stamp.
pub(super) fn ownship_from_wire(
    sample: &wire::TelemetrySample,
    received_at: MonotonicNanos,
    planar_pose_is_truth: bool,
) -> Option<OwnshipSample> {
    let acquired_at = sample
        .observed_at
        .as_ref()
        .map_or(received_at, |stamp| MonotonicNanos::from_nanos(stamp.nanos));
    if let Some(truth) = sample.sim_truth.as_deref() {
        return from_sim_truth(truth, acquired_at);
    }
    // A stamped kinematics group is authoritative for this sample: a
    // role the mission has no reading for refuses the sample outright.
    // Falling through to the pose fallback would UPGRADE an unreadable
    // stamped role to simulation truth — the fail-open ADR-0024 breach.
    if let Some(avionics) = sample.avionics.as_ref()
        && avionics.kinematics_stamp.is_some()
    {
        return from_avionics(avionics, acquired_at);
    }
    if planar_pose_is_truth {
        from_pose(sample, acquired_at)
    } else {
        None
    }
}

/// A stamped simulator-truth group; truth without provenance is
/// unconsumable and yields nothing.
fn from_sim_truth(
    truth: &wire::SimTruthState,
    acquired_at: MonotonicNanos,
) -> Option<OwnshipSample> {
    let stamp = truth.stamp.as_ref()?;
    Some(OwnshipSample {
        ned: [
            f64::from(truth.pos_n_m),
            f64::from(truth.pos_e_m),
            f64::from(truth.pos_d_m),
        ],
        ned_velocity: [
            f64::from(truth.vel_n_mps),
            f64::from(truth.vel_e_mps),
            f64::from(truth.vel_d_mps),
        ],
        yaw_rad: Some(quat_yaw(
            truth.quat_w,
            truth.quat_x,
            truth.quat_y,
            truth.quat_z,
        )),
        role: role_from_wire(stamp.role)?,
        acquired_at,
        sequence: stamp.sequence,
    })
}

/// A stamped avionics kinematics group under its declared role; yaw comes
/// from the attitude group when one rides along and is absent otherwise —
/// a substituted zero would read as a due-north heading.
fn from_avionics(
    avionics: &wire::AvionicsState,
    acquired_at: MonotonicNanos,
) -> Option<OwnshipSample> {
    let stamp = avionics.kinematics_stamp.as_ref()?;
    let yaw_rad = avionics.attitude_stamp.as_ref().map(|_| {
        quat_yaw(
            avionics.quat_w,
            avionics.quat_x,
            avionics.quat_y,
            avionics.quat_z,
        )
    });
    Some(OwnshipSample {
        ned: [
            f64::from(avionics.pos_n_m),
            f64::from(avionics.pos_e_m),
            f64::from(avionics.pos_d_m),
        ],
        ned_velocity: [
            f64::from(avionics.vel_n_mps),
            f64::from(avionics.vel_e_mps),
            f64::from(avionics.vel_d_mps),
        ],
        yaw_rad,
        role: role_from_wire(stamp.role)?,
        acquired_at,
        sequence: stamp.sequence,
    })
}

/// The planar fallback, reachable only when the spawn site declared the
/// host's planar poses to be the simulator's own state (the deterministic
/// reference adapter, which has no estimator to launder). The
/// simulation-truth role is that declaration, not an inference; depth and
/// vertical rate are synthesized as zero and yaw is the planar heading.
fn from_pose(sample: &wire::TelemetrySample, acquired_at: MonotonicNanos) -> Option<OwnshipSample> {
    let pose = sample.pose.as_ref()?;
    let speed = sample
        .velocity
        .as_ref()
        .map_or(0.0, |velocity| f64::from(velocity.linear_x_mps));
    let heading = f64::from(pose.heading_rad);
    let (sin, cos) = heading.sin_cos();
    Some(OwnshipSample {
        ned: [f64::from(pose.x_m), f64::from(pose.y_m), 0.0],
        ned_velocity: [speed * cos, speed * sin, 0.0],
        yaw_rad: Some(heading),
        role: TruthRole::SimulationTruth,
        acquired_at,
        sequence: sample.tick.as_ref().map_or(0, |tick| tick.value as u32),
    })
}

/// Maps a wire source role onto the mission vocabulary; roles the mission
/// has no reading for (video, payload device) yield nothing.
fn role_from_wire(role: i32) -> Option<TruthRole> {
    match wire::SourceRole::try_from(role).ok()? {
        wire::SourceRole::SimulationTruth => Some(TruthRole::SimulationTruth),
        wire::SourceRole::OperationalEstimate => Some(TruthRole::OperationalEstimate),
        wire::SourceRole::FcState => Some(TruthRole::FcState),
        // Guidance output is the mission's own plan-relative product
        // (ADR-0024/0031): reading it back as an ownship observation
        // would close a loop on the executor's own numbers.
        wire::SourceRole::Unspecified
        | wire::SourceRole::VideoCapture
        | wire::SourceRole::PayloadDevice
        | wire::SourceRole::NavigationSolution => None,
    }
}

/// Heading (yaw about NED down) of a body-FRD-to-NED quaternion.
fn quat_yaw(w: f32, x: f32, y: f32, z: f32) -> f64 {
    let (w, x, y, z) = (f64::from(w), f64::from(x), f64::from(y), f64::from(z));
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    fn now() -> MonotonicNanos {
        MonotonicNanos::from_nanos(1_000_000)
    }

    fn stamped_kinematics(role: wire::SourceRole) -> wire::TelemetrySample {
        wire::TelemetrySample {
            avionics: Some(wire::AvionicsState {
                baro_alt_m: 0.0,
                baro_stamp: None,
                pos_n_m: 1.0,
                pos_e_m: 2.0,
                pos_d_m: -3.0,
                kinematics_stamp: Some(wire::MeasurementStamp {
                    role: role as i32,
                    sequence: 7,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            // A pose rides along so a fall-through would have something
            // to upgrade — the tests below prove it never does.
            pose: Some(wire::Pose2d {
                x_m: 9.0,
                y_m: 9.0,
                heading_rad: 1.0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_unreadable_stamped_role_is_refused_not_upgraded_to_pose_truth() {
        let sample = stamped_kinematics(wire::SourceRole::NavigationSolution);
        assert_eq!(ownship_from_wire(&sample, now(), true), None);
        assert_eq!(ownship_from_wire(&sample, now(), false), None);
    }

    #[test]
    fn a_planar_pose_is_truth_only_by_declaration() {
        let sample = wire::TelemetrySample {
            pose: Some(wire::Pose2d {
                x_m: 4.0,
                y_m: 5.0,
                heading_rad: 0.5,
            }),
            ..Default::default()
        };
        let declared =
            ownship_from_wire(&sample, now(), true).expect("declared planar truth converts");
        assert_eq!(declared.role, TruthRole::SimulationTruth);
        assert_eq!(declared.yaw_rad, Some(0.5));
        assert_eq!(ownship_from_wire(&sample, now(), false), None);
    }

    #[test]
    fn kinematics_without_an_attitude_group_carries_no_yaw() {
        let sample = stamped_kinematics(wire::SourceRole::SimulationTruth);
        let ownship = ownship_from_wire(&sample, now(), false).expect("truth role converts");
        assert_eq!(
            ownship.yaw_rad, None,
            "a substituted zero would read as due north"
        );
        assert_eq!(ownship.ned, [1.0, 2.0, -3.0]);
    }

    #[test]
    fn an_attitude_group_supplies_the_yaw() {
        let mut sample = stamped_kinematics(wire::SourceRole::SimulationTruth);
        let avionics = sample.avionics.as_mut().expect("fixture carries avionics");
        avionics.quat_w = 1.0;
        avionics.attitude_stamp = Some(wire::MeasurementStamp::default());
        let ownship = ownship_from_wire(&sample, now(), false).expect("truth role converts");
        assert_eq!(
            ownship.yaw_rad,
            Some(0.0),
            "identity quaternion is a zero yaw"
        );
    }
}
