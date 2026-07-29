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
/// the simulation-truth group, then stamped avionics kinematics, then the
/// planar pose. `received_at` is the fallback acquisition time when the
/// sample carries no host publication stamp.
pub(super) fn ownship_from_wire(
    sample: &wire::TelemetrySample,
    received_at: MonotonicNanos,
) -> Option<OwnshipSample> {
    let acquired_at = sample
        .observed_at
        .as_ref()
        .map_or(received_at, |stamp| MonotonicNanos::from_nanos(stamp.nanos));
    if let Some(truth) = sample.sim_truth.as_deref() {
        return from_sim_truth(truth, acquired_at);
    }
    if let Some(avionics) = sample.avionics.as_ref()
        && let Some(ownship) = from_avionics(avionics, acquired_at)
    {
        return Some(ownship);
    }
    from_pose(sample, acquired_at)
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
        yaw_rad: quat_yaw(truth.quat_w, truth.quat_x, truth.quat_y, truth.quat_z),
        role: role_from_wire(stamp.role)?,
        acquired_at,
        sequence: stamp.sequence,
    })
}

/// A stamped avionics kinematics group under its declared role; yaw comes
/// from the attitude group when one rides along, zero otherwise (the
/// mission engine refuses non-truth roles before yaw ever matters).
fn from_avionics(
    avionics: &wire::AvionicsState,
    acquired_at: MonotonicNanos,
) -> Option<OwnshipSample> {
    let stamp = avionics.kinematics_stamp.as_ref()?;
    let yaw_rad = avionics.attitude_stamp.as_ref().map_or(0.0, |_| {
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

/// The planar fallback. An UNSTAMPED planar pose is only published by the
/// deterministic reference simulation, whose pose IS the simulator's own
/// state (it has no estimator to launder), so it carries the
/// simulation-truth role by construction; depth and vertical rate are
/// synthesized as zero and yaw is the planar heading.
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
        yaw_rad: heading,
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
