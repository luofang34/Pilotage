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
use pilotage_geo::{HorizontalDatum, VerticalDatum};
use pilotage_mission::{OwnshipAltitude, OwnshipSample, TruthRole};
use pilotage_protocol::wire;

/// Estimator authorization bit for position.
const VALID_POSITION: u32 = 0b0100;
/// Estimate quality the vehicle grades unusable.
const QUALITY_UNUSABLE: u32 = 2;

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

/// Converts one wire telemetry sample into an ownship sample under the
/// vehicle-estimate source: the vehicle's own fix becomes NED from the
/// mission anchor through `ned_of`, and the stamped kinematics group
/// supplies the velocity. The sample is admitted only when the vehicle's
/// estimator status authorizes its position and grades it usable, and
/// when both the fix and the kinematics ride the estimate role. Yaw comes
/// from an authorized canonical attitude with the same acquisition stamp.
/// A reported Euler value has no verified navigation reference.
pub(super) fn estimate_from_wire(
    sample: &wire::TelemetrySample,
    received_at: MonotonicNanos,
    ned_of: impl Fn(f64, f64, OwnshipAltitude) -> [f64; 3],
) -> Option<OwnshipSample> {
    let acquired_at = sample
        .observed_at
        .as_ref()
        .map_or(received_at, |stamp| MonotonicNanos::from_nanos(stamp.nanos));
    let avionics = sample.avionics.as_ref()?;
    avionics.estimator_status_stamp.as_ref()?;
    if avionics.valid_flags & VALID_POSITION == 0 || avionics.quality >= QUALITY_UNUSABLE {
        return None;
    }
    let kinematics = avionics.kinematics_stamp.as_ref()?;
    let fix_stamp = avionics.geodetic_stamp.as_ref()?;
    if role_from_wire(kinematics.role)? != TruthRole::OperationalEstimate
        || role_from_wire(fix_stamp.role)? != TruthRole::OperationalEstimate
    {
        return None;
    }
    let fix = avionics.geodetic.as_ref()?;
    let datum = HorizontalDatum::from_u8(u8::try_from(fix.horizontal_datum).ok()?)?;
    if datum != HorizontalDatum::Wgs84 {
        return None;
    }
    let altitude = estimate_altitude(fix)?;
    let yaw_rad = authorized_yaw(avionics, fix_stamp)?;
    Some(OwnshipSample {
        ned: ned_of(
            fix.latitude_deg.to_radians(),
            fix.longitude_deg.to_radians(),
            altitude,
        ),
        ned_velocity: [
            f64::from(avionics.vel_n_mps),
            f64::from(avionics.vel_e_mps),
            f64::from(avionics.vel_d_mps),
        ],
        yaw_rad: Some(yaw_rad),
        role: TruthRole::OperationalEstimate,
        acquired_at,
        sequence: fix_stamp.sequence,
    })
}

/// The vertical reading of a wire fix the mission can place: an
/// ellipsoidal height directly, a declared local-relative height as height
/// above the anchor. Every other datum has no declared conversion here.
fn estimate_altitude(fix: &wire::GeodeticFix) -> Option<OwnshipAltitude> {
    match VerticalDatum::from_u8(u8::try_from(fix.vertical_datum).ok()?)? {
        VerticalDatum::Ellipsoid => Some(OwnshipAltitude::Ellipsoid(fix.height_m)),
        VerticalDatum::LocalRelative if fix.local_origin != 0 => {
            Some(OwnshipAltitude::AboveAnchor(fix.height_m))
        }
        _ => None,
    }
}

// Geographic guidance needs a north-referenced body attitude, not a display value.
fn authorized_yaw(avionics: &wire::AvionicsState, fix: &wire::MeasurementStamp) -> Option<f64> {
    let attitude = avionics.attitude_stamp.as_ref()?;
    if avionics.valid_flags & 1 == 0 || attitude != fix {
        return None;
    }
    let q = [
        avionics.quat_w,
        avionics.quat_x,
        avionics.quat_y,
        avionics.quat_z,
    ];
    let norm = q.iter().map(|value| value * value).sum::<f32>();
    if q.iter().any(|value| !value.is_finite()) || (norm - 1.0).abs() > 1e-3 {
        return None;
    }
    Some(quat_yaw(q[0], q[1], q[2], q[3]))
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
mod tests;
