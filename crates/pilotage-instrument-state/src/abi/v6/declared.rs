//! Payload codecs for the unstamped, declared groups.
//!
//! These groups carry no age: they are pilot/UI state or per-frame trust
//! metadata. The encoder omits a group exactly when it equals its
//! fail-closed default, so an absent tag and the default state are the
//! same thing on both sides of the wire.
//!
//! Layouts (payload-relative offsets, LE):
//!
//! | group | layout | len |
//! |-------|--------|----:|
//! | selections | heading bug f32; bug ref u8; alt sel class u8; alt sel model u8; 0; alt sel f32; alt sel origin u32; baro sel f32 | 20 |
//! | trust | quality u8; coherence u8; valid flags u16; generation u32 | 8 |
//! | altitude | class u8; geoid model u8; 0×2; sample f32; origin u32 | 12 |
//!
//! Trust valid-flag bits: 0 attitude, 1 rates, 2 position, 3 velocity,
//! 4 heading, 5 variation, 6 turn, 7 slip; bits 8–15 spare (zero).

use super::{AbiError, get_f32, get_u8, get_u16, get_u32, put_f32, put_u8, put_u16, put_u32};
use crate::abi::{opt, or_nan};
use crate::aircraft::{
    AircraftState, EstimateQuality, Selections, SnapshotCoherence, SnapshotMeta, ValidFlags,
};
use crate::altitude::{AltitudeClass, AltitudeDeclaration, GeoidModelId, OriginId};
use crate::heading::HeadingReference;

fn sized(out: &mut [u8], len: usize) -> Result<&mut [u8], AbiError> {
    out.get_mut(..len).ok_or(AbiError::Truncated)
}

pub(super) fn decode_selections(state: &mut AircraftState, p: &[u8]) {
    state.selections = Selections {
        heading_bug_rad: get_f32(p, 0),
        heading_bug_reference: HeadingReference::from_u8(get_u8(p, 4)),
        altitude_sel_class: AltitudeClass::from_u8(get_u8(p, 5)),
        altitude_sel_model: GeoidModelId(get_u8(p, 6)),
        altitude_sel_m: opt(get_f32(p, 8)),
        altitude_sel_origin: OriginId(get_u32(p, 12)),
        baro_sel_hpa: opt(get_f32(p, 16)),
    };
}

pub(super) fn encode_selections(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if state.selections == Selections::default() {
        return Ok(None);
    }
    let p = sized(out, 20)?;
    let sel = &state.selections;
    put_f32(p, 0, sel.heading_bug_rad);
    put_u8(p, 4, sel.heading_bug_reference.to_u8());
    put_u8(p, 5, sel.altitude_sel_class.to_u8());
    put_u8(p, 6, sel.altitude_sel_model.0);
    put_u8(p, 7, 0);
    put_f32(p, 8, or_nan(sel.altitude_sel_m));
    put_u32(p, 12, sel.altitude_sel_origin.0);
    put_f32(p, 16, or_nan(sel.baro_sel_hpa));
    Ok(Some(20))
}

pub(super) fn decode_trust(state: &mut AircraftState, p: &[u8]) {
    state.quality = match get_u8(p, 0) {
        0 => EstimateQuality::Good,
        1 => EstimateQuality::Degraded,
        2 => EstimateQuality::Unusable,
        _ => EstimateQuality::Unknown,
    };
    let coherence = match get_u8(p, 1) {
        0 => SnapshotCoherence::Insufficient,
        1 => SnapshotCoherence::Coherent,
        2 => SnapshotCoherence::ExcessiveSkew,
        _ => SnapshotCoherence::Unknown,
    };
    let flags = get_u16(p, 2);
    state.valid = ValidFlags {
        attitude: flags & 0x0001 != 0,
        rates: flags & 0x0002 != 0,
        position: flags & 0x0004 != 0,
        velocity: flags & 0x0008 != 0,
        heading: flags & 0x0010 != 0,
        variation: flags & 0x0020 != 0,
        turn: flags & 0x0040 != 0,
        slip: flags & 0x0080 != 0,
    };
    state.snapshot = SnapshotMeta {
        generation: get_u32(p, 4),
        coherence,
    };
}

pub(super) fn encode_trust(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    let default_trust = state.quality == EstimateQuality::default()
        && state.valid == ValidFlags::default()
        && state.snapshot == SnapshotMeta::default();
    if default_trust {
        return Ok(None);
    }
    let p = sized(out, 8)?;
    put_u8(
        p,
        0,
        match state.quality {
            EstimateQuality::Good => 0,
            EstimateQuality::Degraded => 1,
            EstimateQuality::Unusable => 2,
            EstimateQuality::Unknown => 255,
        },
    );
    put_u8(
        p,
        1,
        match state.snapshot.coherence {
            SnapshotCoherence::Insufficient => 0,
            SnapshotCoherence::Coherent => 1,
            SnapshotCoherence::ExcessiveSkew => 2,
            SnapshotCoherence::Unknown => 255,
        },
    );
    let v = &state.valid;
    let flags = u16::from(v.attitude)
        | (u16::from(v.rates) << 1)
        | (u16::from(v.position) << 2)
        | (u16::from(v.velocity) << 3)
        | (u16::from(v.heading) << 4)
        | (u16::from(v.variation) << 5)
        | (u16::from(v.turn) << 6)
        | (u16::from(v.slip) << 7);
    put_u16(p, 2, flags);
    put_u32(p, 4, state.snapshot.generation);
    Ok(Some(8))
}

pub(super) fn decode_altitude(state: &mut AircraftState, p: &[u8]) {
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::from_u8(get_u8(p, 0)),
        geoid_model: GeoidModelId(get_u8(p, 1)),
        sample_m: opt(get_f32(p, 4)),
        origin: OriginId(get_u32(p, 8)),
    };
}

pub(super) fn encode_altitude(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if state.altitude == AltitudeDeclaration::default() {
        return Ok(None);
    }
    let p = sized(out, 12)?;
    put_u8(p, 0, state.altitude.reference_class.to_u8());
    put_u8(p, 1, state.altitude.geoid_model.0);
    put_u8(p, 2, 0);
    put_u8(p, 3, 0);
    put_f32(p, 4, or_nan(state.altitude.sample_m));
    put_u32(p, 8, state.altitude.origin.0);
    Ok(Some(12))
}
