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
    let Some(kin) = latest.kinematics else {
        return TelemetryBatch::default();
    };
    if kin.received_at.elapsed() > WITHHOLD_AFTER {
        return TelemetryBatch::default();
    }
    let attitude = latest
        .attitude
        .filter(|att| att.received_at.elapsed() <= WITHHOLD_AFTER);

    let heading = attitude.map_or(0.0, |att| yaw_of(att.quat_wxyz));
    let speed = f64::from(
        (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1]).sqrt(),
    );
    let avionics = attitude.map(|att| AvionicsSample {
        quat_wxyz: att.quat_wxyz,
        rates_rps: att.rates_rps,
        pos_ned_m: kin.pos_ned_m,
        vel_ned_mps: kin.vel_ned_mps,
        // Aviate's wire subset does not carry its StateValidFlags /
        // EstimateQuality yet (ADR-0018 names the gap); freshness is
        // the only validity dimension this link can honestly claim.
        valid_flags: 0b1111,
        quality: 0,
        arm_state,
    });
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(u64::from(kin.time_boot_ms).wrapping_mul(1_000_000)),
            pose: Pose2d {
                x: f64::from(kin.pos_ned_m[0]),
                y: f64::from(kin.pos_ned_m[1]),
                heading,
            },
            speed,
            avionics,
        }],
    }
}
