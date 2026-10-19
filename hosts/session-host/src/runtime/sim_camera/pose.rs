//! Simulator truth on the wire, reduced to the pose a world-model camera
//! producer renders from.

use pilotage_adapter_api::MeasurementClock;
use pilotage_protocol::wire;
use pilotage_sim_video::wire::{BridgeGimbalPose, BridgeViewPose};

/// Truth `valid_flags` bit for the attitude group.
const ATTITUDE_VALID: u32 = 1 << 0;
/// Truth `valid_flags` bit for the position group.
const POSITION_VALID: u32 = 1 << 2;
/// `pilotage_geo::HorizontalDatum::Wgs84` on the wire, the only datum
/// `BridgeViewPose` states positions in.
const WGS84: u32 = 1;

/// A view pose and the clock its `sim_time_ns` reads, which is the clock
/// the frames rendered from it are captured on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TruthPose {
    /// The pose to send to the producer.
    pub(super) pose: BridgeViewPose,
    /// Clock domain of `pose.sim_time_ns`.
    pub(super) clock: MeasurementClock,
}

/// The pose of a sample's simulator-truth group, or `None` when the sample
/// has no truth a camera can stand on: no truth, no provenance, a role
/// other than simulation truth, an unset attitude or position, no WGS-84
/// geodetic position, or a non-finite value. An estimate is never used in
/// its place, because the camera shows the world as it is.
pub(super) fn truth_pose(sample: &wire::TelemetrySample) -> Option<TruthPose> {
    let truth = sample.sim_truth.as_deref()?;
    let stamp = truth.stamp.as_ref()?;
    if stamp.role() != wire::SourceRole::SimulationTruth {
        return None;
    }
    let both = ATTITUDE_VALID | POSITION_VALID;
    if truth.valid_flags & both != both {
        return None;
    }
    let clock = match stamp.clock() {
        wire::MeasurementClock::Unspecified => return None,
        wire::MeasurementClock::VehicleBoot => MeasurementClock::VehicleBoot,
        wire::MeasurementClock::Simulation => MeasurementClock::Simulation,
        wire::MeasurementClock::HostMonotonic => MeasurementClock::HostMonotonic,
    };
    let geodetic = truth
        .geodetic
        .as_ref()
        .filter(|fix| fix.horizontal_datum == WGS84)?;
    let quat = unit_quaternion([truth.quat_w, truth.quat_x, truth.quat_y, truth.quat_z])?;
    let position = [
        geodetic.latitude_deg,
        geodetic.longitude_deg,
        geodetic.height_m,
    ];
    if !position.iter().all(|value| value.is_finite()) {
        return None;
    }
    let pose = BridgeViewPose {
        sim_time_ns: stamp.acquired_at_ns,
        latitude_deg: geodetic.latitude_deg,
        longitude_deg: geodetic.longitude_deg,
        altitude_m: geodetic.height_m,
        vertical_datum: geodetic.vertical_datum,
        q_w: quat[0],
        q_x: quat[1],
        q_y: quat[2],
        q_z: quat[3],
        gimbal: sample
            .gimbal
            .as_ref()
            .and_then(|gimbal| gimbal_pose(gimbal, yaw_rad(quat))),
    };
    Some(TruthPose { pose, clock })
}

/// The gimbal's pan and tilt, or `None` when its report is unstamped,
/// failed, non-finite, or names no yaw frame. Pan is relative to the body
/// heading, so an Earth-referenced yaw is taken relative to `vehicle_yaw`.
fn gimbal_pose(gimbal: &wire::GimbalAttitude, vehicle_yaw: f64) -> Option<BridgeGimbalPose> {
    gimbal.stamp.as_ref()?;
    if gimbal.failure_flags != 0 {
        return None;
    }
    let quat = unit_quaternion([gimbal.quat_w, gimbal.quat_x, gimbal.quat_y, gimbal.quat_z])?;
    let pan = match gimbal.yaw_frame() {
        wire::GimbalYawFrame::Unspecified => return None,
        wire::GimbalYawFrame::Vehicle => yaw_rad(quat),
        wire::GimbalYawFrame::Earth => wrap_pi(yaw_rad(quat) - vehicle_yaw),
    };
    Some(BridgeGimbalPose {
        pan_rad: pan as f32,
        tilt_rad: pitch_rad(quat) as f32,
        // A gimbal device reports no zoom; the producer starts at its widest
        // detent.
        zoom_detent: 0,
    })
}

/// The quaternion in `f64`, normalized, or `None` when it is not finite or
/// too far from unit length to be an attitude.
fn unit_quaternion(quat: [f32; 4]) -> Option<[f64; 4]> {
    let quat = quat.map(f64::from);
    let norm = quat.iter().map(|value| value * value).sum::<f64>().sqrt();
    if !norm.is_finite() || (norm - 1.0).abs() > 0.01 {
        return None;
    }
    Some(quat.map(|value| value / norm))
}

/// Yaw of a Hamilton (w, x, y, z) rotation in the Z-Y-X order.
fn yaw_rad([w, x, y, z]: [f64; 4]) -> f64 {
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// Pitch of a Hamilton (w, x, y, z) rotation in the Z-Y-X order, positive
/// nose up.
fn pitch_rad([w, x, y, z]: [f64; 4]) -> f64 {
    (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin()
}

/// An angle wrapped into (-pi, pi].
fn wrap_pi(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(std::f64::consts::TAU);
    if wrapped > std::f64::consts::PI {
        wrapped - std::f64::consts::TAU
    } else {
        wrapped
    }
}
