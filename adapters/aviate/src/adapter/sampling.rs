//! Telemetry sampling helpers shared by the adapter's link paths.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{AvionicsSample, Pose2d, TelemetryBatch, TelemetrySample};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::WITHHOLD_AFTER;
use crate::link::LatestAviate;

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

/// The MAVLink-path sampling, unchanged semantics from ADR-0018.
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

    let heading = attitude.map_or(0.0, |att| yaw_of(att.quat_wxyz));
    let pos_ned_m = kinematics.map_or([0.0; 3], |kin| kin.pos_ned_m);
    let vel_ned_mps = kinematics.map_or([0.0; 3], |kin| kin.vel_ned_mps);
    let speed =
        f64::from((vel_ned_mps[0] * vel_ned_mps[0] + vel_ned_mps[1] * vel_ned_mps[1]).sqrt());
    let avionics = Some(AvionicsSample {
        quat_wxyz: attitude.map_or([1.0, 0.0, 0.0, 0.0], |att| att.quat_wxyz),
        rates_rps: attitude.map_or([0.0; 3], |att| att.rates_rps),
        pos_ned_m,
        vel_ned_mps,
        // Aviate's wire subset does not carry its StateValidFlags /
        // EstimateQuality yet (ADR-0018 names the gap); freshness is
        // the only validity dimension this link can honestly claim.
        valid_flags: (u32::from(attitude.is_some()) * 0b0011)
            | (u32::from(kinematics.is_some()) * 0b1100),
        quality: 0,
        arm_state,
        attitude_stamp: attitude.map(|att| att.stamp),
        kinematics_stamp: kinematics.map(|kin| kin.stamp),
    });
    let source_time_ms = kinematics
        .map(|kin| kin.time_boot_ms)
        .or_else(|| attitude.map(|att| att.time_boot_ms))
        .unwrap_or_default();
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(u64::from(source_time_ms).wrapping_mul(1_000_000)),
            pose: Pose2d {
                x: f64::from(pos_ned_m[0]),
                y: f64::from(pos_ned_m[1]),
                heading,
            },
            speed,
            avionics,
        }],
    }
}
