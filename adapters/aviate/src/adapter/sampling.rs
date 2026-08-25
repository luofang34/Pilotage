//! Telemetry sampling helpers shared by the adapter's link paths.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, Pose2d, SourceRole,
    TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::WITHHOLD_AFTER;
use pilotage_mavlink::link::estimator::{QUALITY_DEGRADED, QUALITY_UNUSABLE};
use pilotage_mavlink::link::{AttitudeUpdate, KinematicsUpdate, LinkState};

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

fn attitude_euler_of(q: [f32; 4]) -> Option<[f32; 3]> {
    if !q.iter().all(|component| component.is_finite()) {
        return None;
    }
    let (w, x, y, z) = (q[0], q[1], q[2], q[3]);
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = yaw_of(q) as f32;
    [roll, pitch, yaw]
        .iter()
        .all(|angle| angle.is_finite())
        .then_some([roll, pitch, yaw])
}

/// One coherent operational pose for control seeding.
pub(super) struct CurrentPose {
    /// Roll, pitch, and yaw in radians.
    pub(super) attitude_rad: [f32; 3],
    /// NED position in meters.
    pub(super) pos_ned_m: [f32; 3],
    /// Independently validated NED velocity in meters per second.
    pub(super) velocity_ned_mps: Option<[f32; 3]>,
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

/// The fresh simulation-truth sample, stamped with its own source role so
/// a consumer can never mistake it for the FC's operational estimate.
fn sim_truth_sample(latest: &LinkState) -> Option<pilotage_adapter_api::SimTruthSample> {
    latest
        .sim_truth
        .filter(|truth| truth.received_at.elapsed() <= WITHHOLD_AFTER)
        .map(|truth| pilotage_adapter_api::SimTruthSample {
            geodetic: None,
            quat_wxyz: truth.quat_wxyz,
            pos_ned_m: truth.pos_ned_m,
            vel_ned_mps: truth.vel_ned_mps,
            // Attitude, position, and velocity are all carried;
            // the truth stream has no body-rate report.
            valid_flags: 0b1101,
            stamp: pilotage_adapter_api::MeasurementStamp {
                role: pilotage_adapter_api::SourceRole::SimulationTruth,
                integrity: pilotage_adapter_api::SourceIntegrity::ChecksummedOnly,
                source_id: latest.source_id,
                source_incarnation: latest.source_incarnation,
                source_epoch: 0,
                sequence: truth.sequence,
                acquired_at_ns: truth.time_usec.wrapping_mul(1_000),
                clock: pilotage_adapter_api::MeasurementClock::Simulation,
            },
        })
}

pub(crate) fn mavlink_batch(vehicle: VehicleId, state: &Arc<Mutex<LinkState>>) -> TelemetryBatch {
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
        // The estimate stream carries no geodetic report.
        geodetic: None,
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
        baro: latest
            .baro
            .filter(|update| update.received_at.elapsed() <= WITHHOLD_AFTER)
            .map(|update| pilotage_adapter_api::AvionicsBaroSample {
                pressure_alt_m: update.pressure_alt_m,
                stamp: update.stamp,
            }),
        estimator_status_stamp,
        valid_flags,
        quality,
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
            sim_truth: sim_truth_sample(&latest),
            fc_state: None,
            gimbal: None,
        }],
    }
}

impl super::AviateAdapter {
    /// The vehicle's current measured attitude, NED position, and
    /// independently validated NED velocity,
    /// FROM THE FC OPERATIONAL ESTIMATE ONLY (LINK-04): simulation truth
    /// is never eligible to seed command construction, so without a live
    /// authorized estimate there is no pose and state-dependent control
    /// is rejected instead of borrowing truth.
    ///
    /// Velocity carries its own validity: `None` when the FC did not
    /// declare the velocity group valid or any component is non-finite.
    /// A pose can be usable while velocity is not; a caller must never
    /// infer "stopped" from a missing velocity.
    pub(super) fn current_pose(&mut self) -> Option<CurrentPose> {
        let latest = self.estimate.as_ref()?.state.lock().ok()?;
        let status_stamp = latest.estimator_status_stamp()?;
        let attitude = latest
            .attitude
            .filter(|update| update.received_at.elapsed() <= WITHHOLD_AFTER)
            .filter(|update| update.stamp.role == SourceRole::OperationalEstimate)?;
        let kinematics = latest
            .kinematics
            .filter(|update| update.received_at.elapsed() <= WITHHOLD_AFTER)
            .filter(|update| update.stamp.role == SourceRole::OperationalEstimate)?;
        let current_epoch = latest.source_epoch;
        if status_stamp.source_epoch != current_epoch
            || attitude.stamp.source_epoch != current_epoch
            || kinematics.stamp.source_epoch != current_epoch
            || !measurement_pair_is_coherent(
                attitude,
                kinematics,
                latest.maximum_inter_group_skew_ms,
            )
            || !measurement_pair_supports_pose(attitude, kinematics)
        {
            return None;
        }
        Some(CurrentPose {
            attitude_rad: attitude_euler_of(attitude.quat_wxyz)?,
            pos_ned_m: kinematics.pos_ned_m,
            velocity_ned_mps: validated_velocity(kinematics),
        })
    }
}

/// The kinematics velocity as independently validated data: present only
/// when the FC declared the velocity group valid (bit 3, the same gate the
/// planar speed projection uses) and every component is finite. NaN would
/// otherwise poison downstream comparisons silently — `NaN > threshold` is
/// false, which reads as "stopped".
fn validated_velocity(kinematics: KinematicsUpdate) -> Option<[f32; 3]> {
    let declared_valid = kinematics.valid_flags & 0b1000 != 0;
    let finite = kinematics.vel_ned_mps.iter().all(|v| v.is_finite());
    (declared_valid && finite).then_some(kinematics.vel_ned_mps)
}
