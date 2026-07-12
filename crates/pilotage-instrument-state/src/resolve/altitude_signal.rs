//! Resolution of the datum-qualified altitude and the full-identity
//! selection-compatibility rule (ALT-01).

use crate::aircraft::AircraftState;
use crate::altitude::AltitudeClass;
use crate::signal::{FreshnessPolicy, Sig, SignalStatus};
use crate::units::M_TO_FT;
use crate::validate::StateIntegrity;

use super::{
    BARO_SETTING_TOLERANCE_HPA, ResolvedAltitude, Trust, fault_status, finite, group_freshness,
};

/// Resolves the datum-qualified altitude for the declared class. The
/// non-local classes ride the air-data group's stamp in ABI v3 (they
/// arrive beside it); a dedicated source group would bring its own
/// stamp. A required source that is absent fails the altitude — the
/// value is a quiet zero and nothing substitutes.
pub(super) fn altitude_resolved(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
    pos_status: SignalStatus,
    rel_alt_ft: f32,
) -> ResolvedAltitude {
    let decl = state.altitude;
    let class = decl.reference_class;
    let fault = fault_status(integrity.altitude);
    let sample_ft = decl.sample_m.map(|m| m * M_TO_FT);
    let sample_status = group_freshness(policy, state.air.data.is_some(), state.air.age_ms)
        .worst(trust.quality)
        .worst(trust.coherence)
        .worst(fault);
    let value = match class {
        AltitudeClass::LocalRelative => Sig::with_status(rel_alt_ft, pos_status.worst(fault)),
        AltitudeClass::BaroIndicated
        | AltitudeClass::Pressure
        | AltitudeClass::GeometricMsl
        | AltitudeClass::Agl => match (sample_ft, integrity.altitude) {
            (Some(v), None) => Sig::with_status(v, sample_status),
            _ => Sig::with_status(0.0, SignalStatus::Failed),
        },
        AltitudeClass::Unknown => Sig::with_status(0.0, SignalStatus::Failed),
    };
    let applied = state.air.data.and_then(|air| air.baro_setting_hpa);
    let setting_mismatch = class == AltitudeClass::BaroIndicated
        && matches!(
            (applied, state.selections.baro_sel_hpa),
            (Some(a), Some(s)) if (a - s).abs() > BARO_SETTING_TOLERANCE_HPA
        );
    let bug_compatible = selection_compatible(state, class, setting_mismatch);
    ResolvedAltitude {
        value_ft: finite(value),
        class,
        origin: decl.origin,
        setting_mismatch,
        bug_compatible,
    }
}

/// Whether the pilot's altitude selection shares the displayed datum's
/// COMPLETE reference identity — class equality alone is never
/// compatibility. Local-relative selections must name the same origin;
/// geometric-MSL selections must name the same declared model; a
/// barometric selection's datum is the applied setting, so a disputed
/// setting suppresses the bug; pressure altitude's datum is fully
/// identified by its class (standard atmosphere); AGL carries no source
/// identity in this ABI revision, so class equality is its complete
/// identity today. Anything unknown or incomplete fails closed.
fn selection_compatible(
    state: &AircraftState,
    displayed: AltitudeClass,
    setting_mismatch: bool,
) -> bool {
    if state.selections.altitude_sel_m.is_none() || state.selections.altitude_sel_class != displayed
    {
        return false;
    }
    let decl = state.altitude;
    let selections = state.selections;
    match displayed {
        AltitudeClass::LocalRelative => selections.altitude_sel_origin == decl.origin,
        AltitudeClass::GeometricMsl => {
            selections.altitude_sel_model == decl.geoid_model
                && selections.altitude_sel_model != crate::altitude::GeoidModelId::UNDECLARED
        }
        AltitudeClass::BaroIndicated => !setting_mismatch,
        AltitudeClass::Pressure | AltitudeClass::Agl => true,
        AltitudeClass::Unknown => false,
    }
}
