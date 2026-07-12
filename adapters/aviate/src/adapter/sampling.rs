//! Telemetry sampling helpers shared by the adapter's link paths.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, Pose2d, TelemetryBatch,
    TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::WITHHOLD_AFTER;
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

    let planar = attitude
        .zip(kinematics)
        .filter(|(att, kin)| {
            measurement_pair_is_coherent(*att, *kin, latest.maximum_inter_group_skew_ms)
        })
        .map(|(att, kin)| {
            let speed = f64::from(
                (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1])
                    .sqrt(),
            );
            (
                Pose2d {
                    x: f64::from(kin.pos_ned_m[0]),
                    y: f64::from(kin.pos_ned_m[1]),
                    heading: yaw_of(att.quat_wxyz),
                },
                speed,
            )
        });
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
        // The source messages do not carry estimator validity or quality;
        // freshness is the only validity dimension this link can claim.
        valid_flags: (u32::from(attitude.is_some()) * 0b0011)
            | (u32::from(kinematics.is_some()) * 0b1100),
        quality: 0,
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
            pose: planar.map(|(pose, _)| pose),
            speed: planar.map(|(_, speed)| speed),
            avionics,
        }],
    }
}
