//! Pure numeric/integrity validators applied before display resolution.
//!
//! No non-finite, malformed, unknown-quality, invalid-quaternion, or
//! otherwise untrusted value may resolve `Valid` or influence display
//! geometry. Validators never repair silently: a fault fails its group
//! with a typed reason, and independent group faults stay isolated.

use libm::sqrtf;

use crate::aircraft::{
    AircraftState, EstimateQuality, NavFromTo, NavSource, Selections, SnapshotCoherence,
};
use pilotage_frames::Quat;

/// Largest relative quaternion norm error normalized instead of failed.
///
/// Within the tolerance the quaternion is renormalized (accumulated
/// rounding from an estimator is expected); at zero, gross, or
/// non-finite norm the attitude is unusable and fails instead.
pub const QUAT_NORM_TOLERANCE: f32 = 0.02;

/// Why one group's data cannot be trusted this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupFault {
    /// A mandatory value is NaN or infinite.
    NonFinite,
    /// The attitude quaternion has zero, gross, or non-finite norm.
    QuatNorm,
    /// The source declared a quality level this build does not know.
    UnknownQuality,
    /// An enum field carried a value this build does not know.
    UnknownEnum,
    /// A reference class requires a source sample, applied setting, or
    /// model identity that was not provided. The group fails; nothing
    /// substitutes.
    SourceAbsent,
}

/// Per-group validation results; `None` means the group's received data
/// passed. Groups without data are not validated (absence is `Missing`,
/// not a fault).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateIntegrity {
    /// Attitude quaternion fault.
    pub attitude: Option<GroupFault>,
    /// Body-rates fault.
    pub rates: Option<GroupFault>,
    /// NED position fault.
    pub position: Option<GroupFault>,
    /// NED velocity fault.
    pub velocity: Option<GroupFault>,
    /// Air-data fault.
    pub air: Option<GroupFault>,
    /// Navigation-guidance fault.
    pub nav: Option<GroupFault>,
    /// Wind-estimate fault.
    pub wind: Option<GroupFault>,
    /// Pilot-selections fault.
    pub selections: Option<GroupFault>,
    /// Source quality was undeclared or unknown (taints every estimate
    /// group, matching how quality itself combines).
    pub quality: Option<GroupFault>,
    /// Snapshot coherence carried an unknown wire value.
    pub coherence: Option<GroupFault>,
    /// Datum-qualified altitude fault: unknown reference class, missing
    /// required source, undeclared model, or non-finite sample.
    pub altitude: Option<GroupFault>,
}

fn all_finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn opt_finite(value: Option<f32>) -> bool {
    value.is_none_or(f32::is_finite)
}

/// Validates the attitude quaternion: every component finite and the
/// norm within [`QUAT_NORM_TOLERANCE`] of unity. Returns the normalized
/// quaternion; never repairs a gross error.
pub fn validate_quat(quat: Quat) -> Result<Quat, GroupFault> {
    if !all_finite(&[quat.w, quat.x, quat.y, quat.z]) {
        return Err(GroupFault::NonFinite);
    }
    let norm = sqrtf(quat.w * quat.w + quat.x * quat.x + quat.y * quat.y + quat.z * quat.z);
    if !norm.is_finite() || (norm - 1.0).abs() > QUAT_NORM_TOLERANCE {
        return Err(GroupFault::QuatNorm);
    }
    Ok(Quat {
        w: quat.w / norm,
        x: quat.x / norm,
        y: quat.y / norm,
        z: quat.z / norm,
    })
}

fn selections_fault(selections: &Selections) -> Option<GroupFault> {
    let finite = selections.heading_bug_rad.is_finite()
        && opt_finite(selections.altitude_sel_m)
        && opt_finite(selections.baro_sel_hpa);
    if finite {
        None
    } else {
        Some(GroupFault::NonFinite)
    }
}

/// Validates every received group of `state` and reports per-group
/// faults. Absent groups pass (their absence resolves `Missing`); the
/// deterministic worst-of combination in `resolve` folds these faults
/// into each signal's status.
pub fn validate_state(state: &AircraftState) -> StateIntegrity {
    let mut integrity = StateIntegrity::default();

    if let Some(attitude) = &state.attitude.data {
        if let Err(fault) = validate_quat(attitude.quat) {
            integrity.attitude = Some(fault);
        }
        if !all_finite(&attitude.rates_rps) {
            integrity.rates = Some(GroupFault::NonFinite);
        }
    }
    if let Some(kinematics) = &state.kinematics.data {
        if !all_finite(&kinematics.pos_ned_m) {
            integrity.position = Some(GroupFault::NonFinite);
        }
        if !all_finite(&kinematics.vel_ned_mps) {
            integrity.velocity = Some(GroupFault::NonFinite);
        }
    }
    if let Some(air) = &state.air.data
        && !(opt_finite(air.ias_mps) && opt_finite(air.baro_setting_hpa))
    {
        integrity.air = Some(GroupFault::NonFinite);
    }
    if let Some(nav) = &state.nav.data {
        if matches!(nav.source, NavSource::Unknown) || matches!(nav.fromto, NavFromTo::Unknown) {
            integrity.nav = Some(GroupFault::UnknownEnum);
        } else if !(all_finite(&[nav.course_rad, nav.cdi_dots])
            && opt_finite(nav.vdev_dots)
            && opt_finite(nav.dist_nm))
        {
            integrity.nav = Some(GroupFault::NonFinite);
        }
    }
    if let Some(wind) = &state.wind.data
        && !all_finite(&[wind.from_rad, wind.speed_mps])
    {
        integrity.wind = Some(GroupFault::NonFinite);
    }
    integrity.selections = selections_fault(&state.selections);
    if state.quality == EstimateQuality::Unknown {
        integrity.quality = Some(GroupFault::UnknownQuality);
    }
    if state.snapshot.coherence == SnapshotCoherence::Unknown {
        integrity.coherence = Some(GroupFault::UnknownEnum);
    }
    integrity.altitude = altitude_fault(state);
    integrity
}

/// Typed reason a datum-qualified altitude cannot display. Class rules:
/// local-relative needs no sample; barometric indicated needs the sample
/// and the source-applied setting; pressure, geometric MSL, and AGL need
/// the sample; geometric MSL also needs a declared model. An unknown
/// class or a non-finite sample fails outright — no reference is ever
/// guessed and no fallback is ever taken.
fn altitude_fault(state: &AircraftState) -> Option<GroupFault> {
    use crate::altitude::{AltitudeClass, GeoidModelId};
    let decl = &state.altitude;
    if !opt_finite(decl.sample_m) {
        return Some(GroupFault::NonFinite);
    }
    let applied = state.air.data.and_then(|air| air.baro_setting_hpa);
    match decl.reference_class {
        AltitudeClass::LocalRelative => None,
        AltitudeClass::BaroIndicated => {
            if decl.sample_m.is_none() || applied.is_none() {
                Some(GroupFault::SourceAbsent)
            } else {
                None
            }
        }
        AltitudeClass::Pressure | AltitudeClass::Agl => {
            if decl.sample_m.is_none() {
                Some(GroupFault::SourceAbsent)
            } else {
                None
            }
        }
        AltitudeClass::GeometricMsl => {
            if decl.sample_m.is_none() || decl.geoid_model == GeoidModelId::UNDECLARED {
                Some(GroupFault::SourceAbsent)
            } else {
                None
            }
        }
        AltitudeClass::Unknown => Some(GroupFault::UnknownEnum),
    }
}

#[cfg(test)]
mod tests;
