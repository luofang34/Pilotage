//! Telemetry sampling from the shared MAVLink cache: freshness
//! withholding, pair-coherence, authorization masking, and the planar
//! projection — the same discipline as the Aviate adapter, over the
//! standard-status authorization source.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, FcStateSample,
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
    TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use pilotage_mavlink::link::estimator::{QUALITY_DEGRADED, QUALITY_UNUSABLE};
use pilotage_mavlink::{AttitudeUpdate, KinematicsUpdate, LinkState};

use super::WITHHOLD_AFTER;

/// Both stamps must come from the same source identity, epoch, and
/// clock, with acquisition times inside the configured skew budget.
pub(super) fn measurement_pair_is_coherent(
    attitude: AttitudeUpdate,
    kinematics: KinematicsUpdate,
    maximum_inter_group_skew_ms: u32,
) -> bool {
    let a = attitude.stamp;
    let k = kinematics.stamp;
    let identity = a.source_id == k.source_id
        && a.source_incarnation == k.source_incarnation
        && a.source_epoch == k.source_epoch
        && a.clock == k.clock;
    let skew_ns = a.acquired_at_ns.abs_diff(k.acquired_at_ns);
    identity && skew_ns <= u64::from(maximum_inter_group_skew_ms).saturating_mul(1_000_000)
}

/// A pose requires an authorized attitude and an authorized position.
fn measurement_pair_supports_pose(attitude: AttitudeUpdate, kinematics: KinematicsUpdate) -> bool {
    attitude.quality <= QUALITY_DEGRADED
        && kinematics.quality <= QUALITY_DEGRADED
        && attitude.valid_flags & 0b0001 != 0
        && kinematics.valid_flags & 0b0100 != 0
}

/// Yaw (radians clockwise from north) from a body-FRD→world-NED
/// quaternion.
pub(super) fn yaw_of(quat_wxyz: [f32; 4]) -> f64 {
    let [w, x, y, z] = quat_wxyz.map(f64::from);
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

fn effective_authorization(
    attitude: Option<AttitudeUpdate>,
    kinematics: Option<KinematicsUpdate>,
) -> (u32, u32) {
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

pub(super) fn mavlink_batch(vehicle: VehicleId, state: &Arc<Mutex<LinkState>>) -> TelemetryBatch {
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
    let (planar_pose, planar_speed) = match (attitude, kinematics) {
        (Some(att), Some(kin))
            if estimator_status_stamp.is_some()
                && measurement_pair_is_coherent(att, kin, latest.maximum_inter_group_skew_ms)
                && measurement_pair_supports_pose(att, kin) =>
        {
            let pose = pilotage_adapter_api::Pose2d {
                x: f64::from(kin.pos_ned_m[0]),
                y: f64::from(kin.pos_ned_m[1]),
                heading: yaw_of(att.quat_wxyz),
            };
            let speed = f64::from(
                (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1])
                    .sqrt(),
            );
            (Some(pose), Some(speed))
        }
        _ => (None, None),
    };
    let (valid_flags, quality) = if estimator_status_stamp.is_some() {
        effective_authorization(attitude, kinematics)
    } else {
        (0, QUALITY_UNUSABLE)
    };
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
            sim_truth: None,
            fc_state: None,
        }],
    }
}

impl super::Px4Adapter {
    /// The vehicle's current measured yaw (radians clockwise from
    /// north), NED position, and independently validated NED velocity,
    /// from the FC operational estimate only. Velocity carries its own
    /// validity: `None` when the velocity group is not declared valid
    /// or any component is non-finite.
    pub(super) fn current_pose(&mut self) -> Option<(f32, [f32; 3], Option<[f32; 3]>)> {
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
        Some((
            yaw_of(attitude.quat_wxyz) as f32,
            kinematics.pos_ned_m,
            validated_velocity(kinematics),
        ))
    }
}

/// The heartbeat-derived FC state sample: arm reported by the FC's own
/// telemetry, stamped under the FC-state source role.
pub(super) fn fc_state_sample(
    report: Option<super::ArmReport>,
    incarnation: SourceIncarnation,
    started_at: std::time::Instant,
) -> Option<FcStateSample> {
    let report = report?;
    let acquired = report
        .acquired_at
        .checked_duration_since(started_at)
        .unwrap_or_default();
    Some(FcStateSample {
        arm_state: if report.armed { 2 } else { 1 },
        stamp: MeasurementStamp {
            role: SourceRole::FcState,
            // MAVLink frames are CRC-checked but unsigned: checksummed,
            // never authenticated.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: 1,
            source_incarnation: incarnation,
            // A gateway-generated attachment identity cannot observe an
            // FC restart; a source-issued boot identity replaces this
            // constant once the FC publishes one.
            source_epoch: 1,
            sequence: report.sequence,
            acquired_at_ns: u64::try_from(acquired.as_nanos()).unwrap_or(u64::MAX),
            clock: MeasurementClock::HostMonotonic,
        },
    })
}

/// The kinematics velocity as independently validated data: present
/// only when the velocity group is declared valid and finite.
fn validated_velocity(kinematics: KinematicsUpdate) -> Option<[f32; 3]> {
    let declared_valid = kinematics.valid_flags & 0b1000 != 0;
    let finite = kinematics.vel_ned_mps.iter().all(|v| v.is_finite());
    (declared_valid && finite).then_some(kinematics.vel_ned_mps)
}
