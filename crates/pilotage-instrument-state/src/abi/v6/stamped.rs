//! Payload codecs for the age-stamped groups.
//!
//! Every stamped group's payload ends with its `age_ms` (NaN = never).
//! Presence semantics: data decodes only when the age is present, and
//! the encoder omits the tag entirely when the [`Stamped`] carries
//! neither data nor age — absence on the wire IS the never-fed group.
//!
//! Layouts (payload-relative offsets, LE):
//!
//! | group | layout | len |
//! |-------|--------|----:|
//! | attitude | quat w x y z f32×4; rates p q r f32×3; age f32 | 32 |
//! | kinematics | pos NED f32×3; vel NED f32×3; age f32 | 28 |
//! | air | ias f32; baro f32; age f32 | 12 |
//! | nav | source u8; fromto u8; course ref u8; 0; course f32; cdi f32; vdev f32; dist f32; age f32; to ident 9; from ident 9 | 42 |
//! | wind | from f32; speed f32; age f32 | 12 |
//! | heading | reference u8; 0×3; heading f32; age f32 | 12 |
//! | variation | source u8; 0×3; east f32; age f32 | 12 |
//! | dynamics | basis u8; 0×3; turn f32; lateral f32; age f32 | 16 |

use super::{AbiError, get_f32, get_u8, put_f32, put_u8};
use crate::abi::{opt, or_nan};
use crate::aircraft::{AirData, AircraftState, Attitude, Kinematics, NavData, Stamped, Wind};
use crate::aircraft::{NavFromTo, NavSource};
use crate::dynamics::{DynSample, TurnBasis, TurnSample};
use crate::heading::{HeadingReference, HeadingSample, MagneticVariation, VariationSourceId};
use crate::ident::IdentStr;
use pilotage_frames::Quat;

fn absent<T>(stamped: &Stamped<T>) -> bool {
    stamped.data.is_none() && stamped.age_ms.is_none()
}

fn sized(out: &mut [u8], len: usize) -> Result<&mut [u8], AbiError> {
    out.get_mut(..len).ok_or(AbiError::Truncated)
}

pub(super) fn decode_attitude(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 28));
    state.attitude = Stamped {
        data: age.map(|_| Attitude {
            quat: Quat {
                w: get_f32(p, 0),
                x: get_f32(p, 4),
                y: get_f32(p, 8),
                z: get_f32(p, 12),
            },
            rates_rps: [get_f32(p, 16), get_f32(p, 20), get_f32(p, 24)],
        }),
        age_ms: age,
    };
}

pub(super) fn encode_attitude(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.attitude) {
        return Ok(None);
    }
    let p = sized(out, 32)?;
    let att = state.attitude.data.unwrap_or(Attitude {
        quat: Quat::IDENTITY,
        rates_rps: [0.0; 3],
    });
    put_f32(p, 0, att.quat.w);
    put_f32(p, 4, att.quat.x);
    put_f32(p, 8, att.quat.y);
    put_f32(p, 12, att.quat.z);
    put_f32(p, 16, att.rates_rps[0]);
    put_f32(p, 20, att.rates_rps[1]);
    put_f32(p, 24, att.rates_rps[2]);
    put_f32(p, 28, or_nan(state.attitude.age_ms));
    Ok(Some(32))
}

pub(super) fn decode_kinematics(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 24));
    state.kinematics = Stamped {
        data: age.map(|_| Kinematics {
            pos_ned_m: [get_f32(p, 0), get_f32(p, 4), get_f32(p, 8)],
            vel_ned_mps: [get_f32(p, 12), get_f32(p, 16), get_f32(p, 20)],
        }),
        age_ms: age,
    };
}

pub(super) fn encode_kinematics(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.kinematics) {
        return Ok(None);
    }
    let p = sized(out, 28)?;
    let kin = state.kinematics.data.unwrap_or(Kinematics {
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
    });
    for (i, value) in kin
        .pos_ned_m
        .iter()
        .chain(kin.vel_ned_mps.iter())
        .enumerate()
    {
        put_f32(p, i * 4, *value);
    }
    put_f32(p, 24, or_nan(state.kinematics.age_ms));
    Ok(Some(28))
}

pub(super) fn decode_air(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 8));
    state.air = Stamped {
        data: age.map(|_| AirData {
            ias_mps: opt(get_f32(p, 0)),
            baro_setting_hpa: opt(get_f32(p, 4)),
        }),
        age_ms: age,
    };
}

pub(super) fn encode_air(state: &AircraftState, out: &mut [u8]) -> Result<Option<usize>, AbiError> {
    if absent(&state.air) {
        return Ok(None);
    }
    let p = sized(out, 12)?;
    let air = state.air.data.unwrap_or_default();
    put_f32(p, 0, or_nan(air.ias_mps));
    put_f32(p, 4, or_nan(air.baro_setting_hpa));
    put_f32(p, 8, or_nan(state.air.age_ms));
    Ok(Some(12))
}

fn ident_at(p: &[u8], off: usize) -> IdentStr {
    p.get(off..off + 9)
        .and_then(|s| <&[u8; 9]>::try_from(s).ok())
        .map_or(IdentStr::INVALID, IdentStr::from_wire)
}

pub(super) fn decode_nav(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 20));
    // Unknown wire values are preserved as Unknown, never mapped to a
    // benign known value (VAL-01 fail-safe decoding).
    let source = match get_u8(p, 0) {
        0 => NavSource::None,
        1 => NavSource::Gps,
        2 => NavSource::Nav1,
        3 => NavSource::Nav2,
        _ => NavSource::Unknown,
    };
    let fromto = match get_u8(p, 1) {
        0 => NavFromTo::Off,
        1 => NavFromTo::To,
        2 => NavFromTo::From,
        _ => NavFromTo::Unknown,
    };
    state.nav = Stamped {
        data: age.map(|_| NavData {
            source,
            course_rad: get_f32(p, 4),
            cdi_dots: get_f32(p, 8),
            fromto,
            vdev_dots: opt(get_f32(p, 12)),
            dist_nm: opt(get_f32(p, 16)),
            course_reference: HeadingReference::from_u8(get_u8(p, 2)),
            to_ident: ident_at(p, 24),
            from_ident: ident_at(p, 33),
        }),
        age_ms: age,
    };
}

pub(super) fn encode_nav(state: &AircraftState, out: &mut [u8]) -> Result<Option<usize>, AbiError> {
    if absent(&state.nav) {
        return Ok(None);
    }
    let p = sized(out, 42)?;
    let nav = state.nav.data.unwrap_or_default();
    put_u8(
        p,
        0,
        match nav.source {
            NavSource::None => 0,
            NavSource::Gps => 1,
            NavSource::Nav1 => 2,
            NavSource::Nav2 => 3,
            NavSource::Unknown => 255,
        },
    );
    put_u8(
        p,
        1,
        match nav.fromto {
            NavFromTo::Off => 0,
            NavFromTo::To => 1,
            NavFromTo::From => 2,
            NavFromTo::Unknown => 255,
        },
    );
    put_u8(p, 2, nav.course_reference.to_u8());
    put_u8(p, 3, 0);
    put_f32(p, 4, nav.course_rad);
    put_f32(p, 8, nav.cdi_dots);
    put_f32(p, 12, or_nan(nav.vdev_dots));
    put_f32(p, 16, or_nan(nav.dist_nm));
    put_f32(p, 20, or_nan(state.nav.age_ms));
    if let Some(dst) = p.get_mut(24..33) {
        dst.copy_from_slice(&nav.to_ident.to_wire());
    }
    if let Some(dst) = p.get_mut(33..42) {
        dst.copy_from_slice(&nav.from_ident.to_wire());
    }
    Ok(Some(42))
}

pub(super) fn decode_wind(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 8));
    state.wind = Stamped {
        data: age.map(|_| Wind {
            from_rad: get_f32(p, 0),
            speed_mps: get_f32(p, 4),
        }),
        age_ms: age,
    };
}

pub(super) fn encode_wind(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.wind) {
        return Ok(None);
    }
    let p = sized(out, 12)?;
    let wind = state.wind.data.unwrap_or(Wind {
        from_rad: 0.0,
        speed_mps: 0.0,
    });
    put_f32(p, 0, wind.from_rad);
    put_f32(p, 4, wind.speed_mps);
    put_f32(p, 8, or_nan(state.wind.age_ms));
    Ok(Some(12))
}

pub(super) fn decode_heading(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 8));
    state.heading = Stamped {
        data: match (age, opt(get_f32(p, 4))) {
            (Some(_), Some(heading_rad)) => Some(HeadingSample {
                heading_rad,
                reference: HeadingReference::from_u8(get_u8(p, 0)),
            }),
            _ => None,
        },
        age_ms: age,
    };
}

pub(super) fn encode_heading(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.heading) {
        return Ok(None);
    }
    let p = sized(out, 12)?;
    let heading = state.heading.data;
    put_u8(p, 0, heading.map_or(255, |sample| sample.reference.to_u8()));
    put_u8(p, 1, 0);
    put_u8(p, 2, 0);
    put_u8(p, 3, 0);
    put_f32(p, 4, or_nan(heading.map(|sample| sample.heading_rad)));
    put_f32(p, 8, or_nan(state.heading.age_ms));
    Ok(Some(12))
}

pub(super) fn decode_variation(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 8));
    state.variation = Stamped {
        data: match (age, opt(get_f32(p, 4))) {
            (Some(_), Some(east_positive_rad)) => Some(MagneticVariation {
                east_positive_rad,
                source: VariationSourceId(get_u8(p, 0)),
            }),
            _ => None,
        },
        age_ms: age,
    };
}

pub(super) fn encode_variation(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.variation) {
        return Ok(None);
    }
    let p = sized(out, 12)?;
    let variation = state.variation.data;
    put_u8(p, 0, variation.map_or(0, |sample| sample.source.0));
    put_u8(p, 1, 0);
    put_u8(p, 2, 0);
    put_u8(p, 3, 0);
    put_f32(
        p,
        4,
        or_nan(variation.map(|sample| sample.east_positive_rad)),
    );
    put_f32(p, 8, or_nan(state.variation.age_ms));
    Ok(Some(12))
}

pub(super) fn decode_dynamics(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, 12));
    state.dynamics = Stamped {
        data: age.map(|_| DynSample {
            turn: opt(get_f32(p, 4)).map(|rate_rps| TurnSample {
                rate_rps,
                basis: TurnBasis::from_u8(get_u8(p, 0)),
            }),
            lateral_mps2: opt(get_f32(p, 8)),
        }),
        age_ms: age,
    };
}

pub(super) fn encode_dynamics(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.dynamics) {
        return Ok(None);
    }
    let p = sized(out, 16)?;
    let dynamics = state.dynamics.data.unwrap_or_default();
    put_u8(
        p,
        0,
        dynamics.turn.map_or(255, |sample| sample.basis.to_u8()),
    );
    put_u8(p, 1, 0);
    put_u8(p, 2, 0);
    put_u8(p, 3, 0);
    put_f32(p, 4, or_nan(dynamics.turn.map(|sample| sample.rate_rps)));
    put_f32(p, 8, or_nan(dynamics.lateral_mps2));
    put_f32(p, 12, or_nan(state.dynamics.age_ms));
    Ok(Some(16))
}

/// Flight-director payload (16 bytes): mode u8, engagement u8, two
/// reserved zero bytes, commanded pitch f32, commanded roll f32,
/// age f32 — unknown mode or engagement bytes decode to the
/// fail-closed sentinels.
pub(super) fn decode_director(state: &mut AircraftState, p: &[u8]) {
    use crate::director::{FdEngagement, FdMode, FdSample};
    let age = opt(get_f32(p, 12));
    state.director = Stamped {
        data: age.map(|_| FdSample {
            mode: FdMode::from_u8(get_u8(p, 0)),
            engagement: FdEngagement::from_u8(get_u8(p, 1)),
            pitch_cmd_rad: get_f32(p, 4),
            roll_cmd_rad: get_f32(p, 8),
        }),
        age_ms: age,
    };
}

pub(super) fn encode_director(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if absent(&state.director) {
        return Ok(None);
    }
    let p = sized(out, 16)?;
    let director = state.director.data.unwrap_or_default();
    put_u8(p, 0, director.mode.to_u8());
    put_u8(p, 1, director.engagement.to_u8());
    put_u8(p, 2, 0);
    put_u8(p, 3, 0);
    put_f32(p, 4, director.pitch_cmd_rad);
    put_f32(p, 8, director.roll_cmd_rad);
    put_f32(p, 12, or_nan(state.director.age_ms));
    Ok(Some(16))
}
