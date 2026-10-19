//! Telemetry sampling from the shared link cache, for every adapter whose
//! flight controller speaks standard MAVLink: freshness withholding,
//! pair-coherence, authorization masking, the planar projection, and the
//! FC-owned reports (arm state, battery) under their own stamps.
//!
//! One implementation serves every such adapter so a freshness or
//! authorization rule cannot drift between flight-controller families.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, BatterySample, FcStateSample,
    GimbalAttitudeSample, SourceRole, TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use crate::link::estimate_geodetic_fix;
use crate::link::estimator::{QUALITY_DEGRADED, QUALITY_UNUSABLE};
use crate::{AttitudeUpdate, KinematicsUpdate, LinkState};

mod battery;
mod fc_state;
pub use battery::{BATTERY_SOURCE_ID, battery_sample};
pub use fc_state::{ArmReport, ArmTracker, fc_state_sample};

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever.
pub const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Both stamps must come from the same source identity, epoch, and
/// clock, with acquisition times inside the configured skew budget.
#[must_use]
pub fn measurement_pair_is_coherent(
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
#[must_use]
pub fn yaw_of(quat_wxyz: [f32; 4]) -> f64 {
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

/// The planar pose and ground speed an authorized, coherent pair supports.
fn planar_projection(
    pair: Option<(AttitudeUpdate, KinematicsUpdate)>,
    authorized: bool,
    maximum_inter_group_skew_ms: u32,
) -> (Option<pilotage_adapter_api::Pose2d>, Option<f64>) {
    match pair {
        Some((att, kin))
            if authorized
                && measurement_pair_is_coherent(att, kin, maximum_inter_group_skew_ms)
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
    }
}

/// The estimate as one telemetry sample: the avionics group with its
/// authorization, and the planar pose when the pair supports one. Empty
/// when neither group is fresh.
pub fn estimate_batch(vehicle: VehicleId, state: &Arc<Mutex<LinkState>>) -> TelemetryBatch {
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
    let (planar_pose, planar_speed) = planar_projection(
        attitude.zip(kinematics),
        estimator_status_stamp.is_some(),
        latest.maximum_inter_group_skew_ms,
    );
    let (valid_flags, quality) = if estimator_status_stamp.is_some() {
        effective_authorization(attitude, kinematics)
    } else {
        (0, QUALITY_UNUSABLE)
    };
    let avionics = Some(AvionicsSample {
        baro: None,
        // The estimator's own fix, under its own stamp: it advances
        // independently of the local frame beside it.
        geodetic: latest
            .gnss_fix
            .filter(|fix| fix.received_at.elapsed() <= WITHHOLD_AFTER)
            .as_ref()
            .and_then(|fix| estimate_geodetic_fix(fix, &latest)),
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
        magnetic_variation: None,
    });
    let source_time_ms = kinematics
        .map(|kin| kin.time_boot_ms)
        .or_else(|| attitude.map(|att| att.time_boot_ms))
        .unwrap_or_default();
    TelemetryBatch {
        samples: vec![TelemetrySample {
            payload: None,
            vehicle,
            tick: SimTick::new(u64::from(source_time_ms).wrapping_mul(1_000_000)),
            pose: planar_pose,
            speed: planar_speed,
            avionics,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
            battery: None,
        }],
    }
}

/// The vehicle's current measured pose from the FC operational estimate
/// only, as the basis for state-dependent control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OperationalPose {
    /// Yaw, radians clockwise from north.
    pub yaw_rad: f32,
    /// NED position, metres.
    pub pos_ned_m: [f32; 3],
    /// NED velocity, m/s; `None` when the velocity group is not declared
    /// valid or any component is non-finite.
    pub vel_ned_mps: Option<[f32; 3]>,
}

/// The current operational pose, or `None` unless a fresh, coherent,
/// authorized attitude and position pair of the current epoch exists.
#[must_use]
pub fn operational_pose(latest: &LinkState) -> Option<OperationalPose> {
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
        || !measurement_pair_is_coherent(attitude, kinematics, latest.maximum_inter_group_skew_ms)
        || !measurement_pair_supports_pose(attitude, kinematics)
    {
        return None;
    }
    Some(OperationalPose {
        yaw_rad: yaw_of(attitude.quat_wxyz) as f32,
        pos_ned_m: kinematics.pos_ned_m,
        vel_ned_mps: validated_velocity(kinematics),
    })
}

/// The kinematics velocity as independently validated data: present
/// only when the velocity group is declared valid and finite.
fn validated_velocity(kinematics: KinematicsUpdate) -> Option<[f32; 3]> {
    let declared_valid = kinematics.valid_flags & 0b1000 != 0;
    let finite = kinematics.vel_ned_mps.iter().all(|v| v.is_finite());
    (declared_valid && finite).then_some(kinematics.vel_ned_mps)
}

/// Reports that the flight controller sends beside its estimate, each under
/// its own stamp.
#[derive(Debug, Clone, Copy)]
pub struct VehicleReports {
    /// The FC-reported arm state.
    pub fc_state: Option<FcStateSample>,
    /// The gimbal device orientation.
    pub gimbal: Option<GimbalAttitudeSample>,
    /// The primary battery.
    pub battery: Option<BatterySample>,
}

impl VehicleReports {
    /// Whether any report is present.
    #[must_use]
    pub fn any(&self) -> bool {
        self.fc_state.is_some() || self.gimbal.is_some() || self.battery.is_some()
    }

    /// Attaches the reports to every sample of `batch`.
    pub fn attach(self, batch: &mut TelemetryBatch) {
        for sample in &mut batch.samples {
            sample.fc_state = self.fc_state;
            sample.gimbal = self.gimbal;
            sample.battery = self.battery;
        }
    }
}

/// A sample with no avionics group, to carry the vehicle reports alone.
#[must_use]
pub fn report_only_sample(vehicle: VehicleId, tick: SimTick) -> TelemetrySample {
    TelemetrySample {
        payload: None,
        vehicle,
        tick,
        pose: None,
        speed: None,
        avionics: None,
        sim_truth: None,
        fc_state: None,
        gimbal: None,
        battery: None,
    }
}
