//! Telemetry sampling helpers shared by the adapter's link paths.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, Pose2d, TelemetryBatch,
    TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::WITHHOLD_AFTER;
use crate::link::estimator::{QUALITY_DEGRADED, QUALITY_UNUSABLE};
use crate::link::{AttitudeUpdate, KinematicsUpdate, LatestAviate};

/// Yaw extracted from the body→NED quaternion (heading, radians
/// clockwise from north).
pub(crate) fn yaw_of(q: [f32; 4]) -> f64 {
    let (w, x, y, z) = (
        f64::from(q[0]),
        f64::from(q[1]),
        f64::from(q[2]),
        f64::from(q[3]),
    );
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

pub(super) fn measurement_pair_is_coherent(
    attitude: AttitudeUpdate,
    kinematics: KinematicsUpdate,
    maximum_skew_ms: u32,
) -> bool {
    let attitude_stamp = attitude.stamp;
    let kinematics_stamp = kinematics.stamp;
    attitude_stamp.source_id == kinematics_stamp.source_id
        && attitude_stamp.source_incarnation == kinematics_stamp.source_incarnation
        && attitude_stamp.source_epoch == kinematics_stamp.source_epoch
        && attitude_stamp.clock == kinematics_stamp.clock
        && attitude_stamp
            .acquired_at_ns
            .abs_diff(kinematics_stamp.acquired_at_ns)
            <= u64::from(maximum_skew_ms) * 1_000_000
}

pub(super) fn measurement_pair_supports_pose(
    attitude: AttitudeUpdate,
    kinematics: KinematicsUpdate,
) -> bool {
    attitude.quality <= QUALITY_DEGRADED
        && kinematics.quality <= QUALITY_DEGRADED
        && attitude.valid_flags & 0b0001 != 0
        && kinematics.valid_flags & 0b0100 != 0
}

fn planar_projection(
    attitude: Option<AttitudeUpdate>,
    kinematics: Option<KinematicsUpdate>,
    maximum_skew_ms: u32,
    has_authorization: bool,
) -> (Option<Pose2d>, Option<f64>) {
    let coherent_pair = attitude.zip(kinematics).filter(|(att, kin)| {
        has_authorization && measurement_pair_is_coherent(*att, *kin, maximum_skew_ms)
    });
    let pose = coherent_pair
        .filter(|(att, kin)| measurement_pair_supports_pose(*att, *kin))
        .map(|(att, kin)| Pose2d {
            x: f64::from(kin.pos_ned_m[0]),
            y: f64::from(kin.pos_ned_m[1]),
            heading: yaw_of(att.quat_wxyz),
        });
    let speed = coherent_pair
        .filter(|(att, kin)| {
            att.quality <= QUALITY_DEGRADED
                && kin.quality <= QUALITY_DEGRADED
                && kin.valid_flags & 0b1000 != 0
        })
        .map(|(_, kin)| {
            f64::from(
                (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1])
                    .sqrt(),
            )
        });
    (pose, speed)
}

fn effective_authorization(
    attitude: Option<AttitudeUpdate>,
    kinematics: Option<KinematicsUpdate>,
    has_authorization: bool,
) -> (u32, u32) {
    if !has_authorization {
        return (0, QUALITY_UNUSABLE);
    }
    let attitude_flags = attitude
        .filter(|att| att.quality <= QUALITY_DEGRADED)
        .map_or(0, |att| att.valid_flags & 0b0011);
    let kinematics_flags = kinematics
        .filter(|kin| kin.quality <= QUALITY_DEGRADED)
        .map_or(0, |kin| kin.valid_flags & 0b1100);
    let flags = attitude_flags | kinematics_flags;
    let quality = attitude
        .filter(|_| attitude_flags != 0)
        .map(|att| att.quality)
        .into_iter()
        .chain(
            kinematics
                .filter(|_| kinematics_flags != 0)
                .map(|kin| kin.quality),
        )
        .max()
        .unwrap_or(QUALITY_UNUSABLE);
    (flags, quality)
}

pub(crate) fn mavlink_batch(
    vehicle: VehicleId,
    state: &Arc<Mutex<LatestAviate>>,
    arm_state: u32,
) -> TelemetryBatch {
    let Ok(latest) = state.lock() else {
        return TelemetryBatch::default();
    };
    let kinematics = latest
        .kinematics
        .filter(|kin| kin.received_at.elapsed() <= WITHHOLD_AFTER);
    let attitude = latest
        .attitude
        .filter(|att| att.received_at.elapsed() <= WITHHOLD_AFTER);
    if attitude.is_none() && kinematics.is_none() {
        return TelemetryBatch::default();
    }

    let estimator_status_stamp = latest.estimator_status_stamp();
    let has_authorization = estimator_status_stamp.is_some();
    let (planar_pose, planar_speed) = planar_projection(
        attitude,
        kinematics,
        latest.maximum_inter_group_skew_ms,
        has_authorization,
    );
    let (valid_flags, quality) = effective_authorization(attitude, kinematics, has_authorization);
    let avionics = Some(AvionicsSample {
        attitude: attitude.map(|att| AvionicsAttitudeSample {
            quat_wxyz: att.quat_wxyz,
            rates_rps: att.rates_rps,
            stamp: att.stamp,
        }),
        kinematics: kinematics.map(|kin| AvionicsKinematicsSample {
            pos_ned_m: kin.pos_ned_m,
            vel_ned_mps: kin.vel_ned_mps,
            stamp: kin.stamp,
        }),
        estimator_status_stamp,
        valid_flags,
        quality,
        arm_state,
    });
    let source_time_ms = kinematics
        .map(|kin| kin.time_boot_ms)
        .or_else(|| attitude.map(|att| att.time_boot_ms))
        .unwrap_or_default();
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(u64::from(source_time_ms).wrapping_mul(1_000_000)),
            pose: planar_pose,
            speed: planar_speed,
            avionics,
        }],
    }
}
