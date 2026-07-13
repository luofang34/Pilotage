//! Wire codecs for the heading, variation, and dynamics groups.

use crate::aircraft::{AircraftState, Stamped};
use crate::dynamics::{DynSample, TurnBasis, TurnSample};
use crate::heading::{HeadingReference, HeadingSample, MagneticVariation, VariationSourceId};

use super::{AbiError, f32_at, opt, or_nan, put_f32, put_u8, u8_at};

pub(super) fn decode_heading(buf: &[u8]) -> Result<Stamped<HeadingSample>, AbiError> {
    let age = opt(f32_at(buf, 160)?);
    Ok(Stamped {
        data: match (age, opt(f32_at(buf, 156)?)) {
            (Some(_), Some(heading_rad)) => Some(HeadingSample {
                heading_rad,
                reference: HeadingReference::from_u8(u8_at(buf, 152)?),
            }),
            _ => None,
        },
        age_ms: age,
    })
}

pub(super) fn decode_variation(buf: &[u8]) -> Result<Stamped<MagneticVariation>, AbiError> {
    let age = opt(f32_at(buf, 168)?);
    Ok(Stamped {
        data: match (age, opt(f32_at(buf, 164)?)) {
            (Some(_), Some(east_positive_rad)) => Some(MagneticVariation {
                east_positive_rad,
                source: VariationSourceId(u8_at(buf, 153)?),
            }),
            _ => None,
        },
        age_ms: age,
    })
}

pub(super) fn decode_dynamics(buf: &[u8]) -> Result<Stamped<DynSample>, AbiError> {
    let age = opt(f32_at(buf, 184)?);
    Ok(Stamped {
        data: match age {
            Some(_) => Some(DynSample {
                turn: opt(f32_at(buf, 176)?).map(|rate_rps| TurnSample {
                    rate_rps,
                    basis: TurnBasis::from_u8(buf.get(173).copied().unwrap_or(255)),
                }),
                lateral_mps2: opt(f32_at(buf, 180)?),
            }),
            None => None,
        },
        age_ms: age,
    })
}

pub(super) fn encode_heading(state: &AircraftState, buf: &mut [u8]) -> Result<(), AbiError> {
    let heading = state.heading.data;
    put_u8(
        buf,
        152,
        heading.map_or(255, |sample| sample.reference.to_u8()),
    )?;
    let variation = state.variation.data;
    put_u8(buf, 153, variation.map_or(0, |sample| sample.source.0))?;
    put_u8(buf, 154, state.selections.heading_bug_reference.to_u8())?;
    put_f32(buf, 156, or_nan(heading.map(|sample| sample.heading_rad)))?;
    put_f32(buf, 160, or_nan(state.heading.age_ms))?;
    put_f32(
        buf,
        164,
        or_nan(variation.map(|sample| sample.east_positive_rad)),
    )?;
    put_f32(buf, 168, or_nan(state.variation.age_ms))?;
    let flags2 = u8::from(state.valid.heading)
        | (u8::from(state.valid.variation) << 1)
        | (u8::from(state.valid.turn) << 2)
        | (u8::from(state.valid.slip) << 3);
    put_u8(buf, 172, flags2)?;
    let dynamics = state.dynamics.data.unwrap_or_default();
    put_u8(
        buf,
        173,
        dynamics.turn.map_or(255, |sample| sample.basis.to_u8()),
    )?;
    put_u8(buf, 174, 0)?;
    put_u8(buf, 175, 0)?;
    put_f32(
        buf,
        176,
        or_nan(dynamics.turn.map(|sample| sample.rate_rps)),
    )?;
    put_f32(buf, 180, or_nan(dynamics.lateral_mps2))?;
    put_f32(buf, 184, or_nan(state.dynamics.age_ms))?;
    for offset in 188..192 {
        put_u8(buf, offset, 0)?;
    }
    Ok(())
}
