//! Packed little-endian input-state layout shared with non-Rust feeders.
//!
//! The browser writes this block into WASM linear memory each frame; the
//! same bytes could arrive over a serial link to an embedded panel. The
//! layout is versioned by its leading `u32`; changes append fields and
//! bump [`STATE_ABI_VERSION`], never reorder. Optional numeric fields use
//! NaN as "absent"; group ages use NaN as "never received".
//!
//! Offsets (bytes, LE) — the JS writer in `clients/web` mirrors this
//! table exactly:
//!
//! | off | type | field |
//! |----:|------|-------|
//! | 0   | u32  | version (=5) |
//! | 4   | f32×4| attitude quaternion w, x, y, z |
//! | 20  | f32×3| body rates p, q, r (rad/s) |
//! | 32  | f32×3| position NED n, e, d (m) |
//! | 44  | f32×3| velocity NED n, e, d (m/s) |
//! | 56  | f32  | IAS (m/s, NaN absent) |
//! | 60  | f32  | baro setting (hPa, NaN absent) |
//! | 64  | f32×5| ages ms: attitude, kinematics, air, nav, wind (NaN never) |
//! | 84  | u8   | quality (0 good, 1 degraded, 2 unusable; other = unknown, fails) |
//! | 85  | u8   | valid flags (bit0 att, 1 rates, 2 pos, 3 vel; unset = not declared valid) |
//! | 86  | u8   | nav source (0 none, 1 gps, 2 nav1, 3 nav2; other = unknown, fails) |
//! | 87  | u8   | nav from/to (0 off, 1 to, 2 from; other = unknown, fails) |
//! | 88  | f32  | nav course (rad) |
//! | 92  | f32  | nav lateral deviation (dots) |
//! | 96  | f32  | nav vertical deviation (dots, NaN absent) |
//! | 100 | f32  | nav distance (NM, NaN absent) |
//! | 104 | f32  | heading bug (rad) |
//! | 108 | f32  | selected altitude (m, NaN none) |
//! | 112 | f32  | wind from (rad) |
//! | 116 | f32  | wind speed (m/s) |
//! | 120 | u32  | snapshot generation (wrapping) |
//! | 124 | u8   | coherence (0 insufficient, 1 coherent, 2 excessive skew; other = unknown, degrades) |
//! | 125 | u8   | altitude reference class (0 rel, 1 baro, 2 std, 3 msl, 4 agl; other = unknown, fails) |
//! | 126 | u8   | selected-altitude reference class (same coding) |
//! | 127 | u8   | geoid model id (0 = undeclared; required for class 3) |
//! | 128 | f32  | altitude sample (m, NaN absent; classes 1-4) |
//! | 132 | f32  | pilot-selected baro setting (hPa, NaN absent) |
//! | 136 | u32  | local-origin identity |
//! | 140 | u8   | selected-altitude geoid model id (0 = undeclared) |
//! | 141 | u8×3 | reserved, zero |
//! | 144 | u32  | selected-altitude origin identity |
//! | 148 | u8×4 | reserved, zero |
//! | 152 | u8   | heading reference (0 magnetic, 1 true, 2 sim-local-true; other = unknown, fails) |
//! | 153 | u8   | variation source id (0 = undeclared; conversion refuses) |
//! | 154 | u8   | heading-bug reference (same coding; unknown suppresses the bug) |
//! | 155 | u8   | course reference (same coding; unknown suppresses CDI/course) |
//! | 156 | f32  | heading (rad from the declared north, NaN absent) |
//! | 160 | f32  | heading age ms (NaN never) |
//! | 164 | f32  | magnetic variation (rad, east positive, NaN absent) |
//! | 168 | f32  | variation age ms (NaN never) |
//! | 172 | u8   | valid flags 2 (bit0 heading, bit1 variation, bit2 turn, bit3 slip) |
//! | 173 | u8   | turn basis (0 heading rate, 1 track rate; other = unknown, fails) |
//! | 174 | u8×2 | reserved, zero |
//! | 176 | f32  | turn rate (rad/s, positive right, NaN absent) |
//! | 180 | f32  | lateral specific force (m/s², body +Y right, NaN absent) |
//! | 184 | f32  | dynamics age ms (NaN never) |
//! | 188 | u8×4 | reserved, zero |

use crate::aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, SnapshotCoherence, SnapshotMeta, Stamped, ValidFlags, Wind,
};
use crate::altitude::{AltitudeClass, AltitudeDeclaration, GeoidModelId, OriginId};
use crate::heading::HeadingReference;
use pilotage_frames::Quat;

/// Version stamped in the block's first four bytes.
pub const STATE_ABI_VERSION: u32 = 5;

/// Size of the packed block in bytes.
pub const STATE_ABI_SIZE: usize = 192;

/// Why a state block failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiError {
    /// The buffer is smaller than [`STATE_ABI_SIZE`].
    Truncated,
    /// The version field is one this decoder does not read.
    BadVersion {
        /// The version found.
        found: u32,
    },
}

pub(crate) fn f32_at(buf: &[u8], off: usize) -> Result<f32, AbiError> {
    let b = buf.get(off..off + 4).ok_or(AbiError::Truncated)?;
    Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u8_at(buf: &[u8], off: usize) -> Result<u8, AbiError> {
    buf.get(off).copied().ok_or(AbiError::Truncated)
}

pub(crate) fn opt(v: f32) -> Option<f32> {
    if v.is_nan() { None } else { Some(v) }
}

/// Decodes a packed state block.
pub fn decode_state(buf: &[u8]) -> Result<AircraftState, AbiError> {
    let vb = buf.get(0..4).ok_or(AbiError::Truncated)?;
    let version = u32::from_le_bytes([vb[0], vb[1], vb[2], vb[3]]);
    if version != STATE_ABI_VERSION {
        return Err(AbiError::BadVersion { found: version });
    }
    if buf.len() < STATE_ABI_SIZE {
        return Err(AbiError::Truncated);
    }

    let att_age = opt(f32_at(buf, 64)?);
    let kin_age = opt(f32_at(buf, 68)?);
    let air_age = opt(f32_at(buf, 72)?);
    let nav_age = opt(f32_at(buf, 76)?);
    let wind_age = opt(f32_at(buf, 80)?);

    let attitude = Stamped {
        data: match att_age {
            Some(_) => Some(Attitude {
                quat: Quat {
                    w: f32_at(buf, 4)?,
                    x: f32_at(buf, 8)?,
                    y: f32_at(buf, 12)?,
                    z: f32_at(buf, 16)?,
                },
                rates_rps: [f32_at(buf, 20)?, f32_at(buf, 24)?, f32_at(buf, 28)?],
            }),
            None => None,
        },
        age_ms: att_age,
    };

    let kinematics = Stamped {
        data: match kin_age {
            Some(_) => Some(Kinematics {
                pos_ned_m: [f32_at(buf, 32)?, f32_at(buf, 36)?, f32_at(buf, 40)?],
                vel_ned_mps: [f32_at(buf, 44)?, f32_at(buf, 48)?, f32_at(buf, 52)?],
            }),
            None => None,
        },
        age_ms: kin_age,
    };

    let air = Stamped {
        data: match air_age {
            Some(_) => Some(AirData {
                ias_mps: opt(f32_at(buf, 56)?),
                baro_setting_hpa: opt(f32_at(buf, 60)?),
            }),
            None => None,
        },
        age_ms: air_age,
    };

    // Unknown wire values are preserved as Unknown, never mapped to a
    // benign known value: guidance from an unidentifiable source must
    // fail, not masquerade as no-source (VAL-01 fail-safe decoding).
    let source = match u8_at(buf, 86)? {
        0 => NavSource::None,
        1 => NavSource::Gps,
        2 => NavSource::Nav1,
        3 => NavSource::Nav2,
        _ => NavSource::Unknown,
    };
    let fromto = match u8_at(buf, 87)? {
        0 => NavFromTo::Off,
        1 => NavFromTo::To,
        2 => NavFromTo::From,
        _ => NavFromTo::Unknown,
    };
    let nav = Stamped {
        data: match nav_age {
            Some(_) => Some(NavData {
                source,
                course_rad: f32_at(buf, 88)?,
                cdi_dots: f32_at(buf, 92)?,
                fromto,
                vdev_dots: opt(f32_at(buf, 96)?),
                dist_nm: opt(f32_at(buf, 100)?),
                course_reference: HeadingReference::from_u8(u8_at(buf, 155)?),
            }),
            None => None,
        },
        age_ms: nav_age,
    };

    let wind = Stamped {
        data: match wind_age {
            Some(_) => Some(Wind {
                from_rad: f32_at(buf, 112)?,
                speed_mps: f32_at(buf, 116)?,
            }),
            None => None,
        },
        age_ms: wind_age,
    };

    let quality = decode_quality(buf)?;
    let valid = decode_valid_flags(buf)?;
    let coherence = match u8_at(buf, 124)? {
        0 => SnapshotCoherence::Insufficient,
        1 => SnapshotCoherence::Coherent,
        2 => SnapshotCoherence::ExcessiveSkew,
        _ => SnapshotCoherence::Unknown,
    };

    Ok(AircraftState {
        attitude,
        kinematics,
        air,
        nav,
        wind,
        selections: decode_selections(buf)?,
        quality,
        valid,
        snapshot: SnapshotMeta {
            generation: u32_at(buf, 120)?,
            coherence,
        },
        altitude: decode_altitude(buf)?,
        heading: decode_heading(buf)?,
        variation: decode_variation(buf)?,
        dynamics: decode_dynamics(buf)?,
    })
}

fn decode_valid_flags(buf: &[u8]) -> Result<ValidFlags, AbiError> {
    let flags = u8_at(buf, 85)?;
    let flags2 = u8_at(buf, 172)?;
    Ok(ValidFlags {
        attitude: flags & 0b0001 != 0,
        rates: flags & 0b0010 != 0,
        position: flags & 0b0100 != 0,
        velocity: flags & 0b1000 != 0,
        heading: flags2 & 0b0001 != 0,
        variation: flags2 & 0b0010 != 0,
        turn: flags2 & 0b0100 != 0,
        slip: flags2 & 0b1000 != 0,
    })
}

fn decode_quality(buf: &[u8]) -> Result<EstimateQuality, AbiError> {
    Ok(match u8_at(buf, 84)? {
        0 => EstimateQuality::Good,
        1 => EstimateQuality::Degraded,
        2 => EstimateQuality::Unusable,
        _ => EstimateQuality::Unknown,
    })
}

fn decode_selections(buf: &[u8]) -> Result<Selections, AbiError> {
    Ok(Selections {
        heading_bug_rad: f32_at(buf, 104)?,
        heading_bug_reference: HeadingReference::from_u8(u8_at(buf, 154)?),
        altitude_sel_m: opt(f32_at(buf, 108)?),
        altitude_sel_class: AltitudeClass::from_u8(u8_at(buf, 126)?),
        altitude_sel_origin: OriginId(u32_at(buf, 144)?),
        altitude_sel_model: GeoidModelId(u8_at(buf, 140)?),
        baro_sel_hpa: opt(f32_at(buf, 132)?),
    })
}

fn decode_altitude(buf: &[u8]) -> Result<AltitudeDeclaration, AbiError> {
    Ok(AltitudeDeclaration {
        reference_class: AltitudeClass::from_u8(u8_at(buf, 125)?),
        sample_m: opt(f32_at(buf, 128)?),
        geoid_model: GeoidModelId(u8_at(buf, 127)?),
        origin: OriginId(u32_at(buf, 136)?),
    })
}

pub(crate) fn u32_at(buf: &[u8], off: usize) -> Result<u32, AbiError> {
    let b = buf.get(off..off + 4).ok_or(AbiError::Truncated)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn put_f32(buf: &mut [u8], off: usize, v: f32) -> Result<(), AbiError> {
    let b = buf.get_mut(off..off + 4).ok_or(AbiError::Truncated)?;
    b.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

pub(crate) fn put_u8(buf: &mut [u8], off: usize, v: u8) -> Result<(), AbiError> {
    *buf.get_mut(off).ok_or(AbiError::Truncated)? = v;
    Ok(())
}

pub(crate) fn put_u32(buf: &mut [u8], off: usize, v: u32) -> Result<(), AbiError> {
    let b = buf.get_mut(off..off + 4).ok_or(AbiError::Truncated)?;
    b.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

pub(crate) fn or_nan(v: Option<f32>) -> f32 {
    v.unwrap_or(f32::NAN)
}

/// Encodes a state into a packed block (the Rust-feeder / test mirror of
/// the JS writer).
pub fn encode_state(state: &AircraftState, buf: &mut [u8]) -> Result<(), AbiError> {
    if buf.len() < STATE_ABI_SIZE {
        return Err(AbiError::Truncated);
    }
    let vb = buf.get_mut(0..4).ok_or(AbiError::Truncated)?;
    vb.copy_from_slice(&STATE_ABI_VERSION.to_le_bytes());

    let att = state.attitude.data.unwrap_or(Attitude {
        quat: Quat::IDENTITY,
        rates_rps: [0.0; 3],
    });
    put_f32(buf, 4, att.quat.w)?;
    put_f32(buf, 8, att.quat.x)?;
    put_f32(buf, 12, att.quat.y)?;
    put_f32(buf, 16, att.quat.z)?;
    put_f32(buf, 20, att.rates_rps[0])?;
    put_f32(buf, 24, att.rates_rps[1])?;
    put_f32(buf, 28, att.rates_rps[2])?;

    let kin = state.kinematics.data.unwrap_or(Kinematics {
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
    });
    put_f32(buf, 32, kin.pos_ned_m[0])?;
    put_f32(buf, 36, kin.pos_ned_m[1])?;
    put_f32(buf, 40, kin.pos_ned_m[2])?;
    put_f32(buf, 44, kin.vel_ned_mps[0])?;
    put_f32(buf, 48, kin.vel_ned_mps[1])?;
    put_f32(buf, 52, kin.vel_ned_mps[2])?;

    let air = state.air.data.unwrap_or_default();
    put_f32(buf, 56, or_nan(air.ias_mps))?;
    put_f32(buf, 60, or_nan(air.baro_setting_hpa))?;

    put_f32(buf, 64, or_nan(state.attitude.age_ms))?;
    put_f32(buf, 68, or_nan(state.kinematics.age_ms))?;
    put_f32(buf, 72, or_nan(state.air.age_ms))?;
    put_f32(buf, 76, or_nan(state.nav.age_ms))?;
    put_f32(buf, 80, or_nan(state.wind.age_ms))?;

    encode_quality(state, buf)?;
    let flags = u8::from(state.valid.attitude)
        | (u8::from(state.valid.rates) << 1)
        | (u8::from(state.valid.position) << 2)
        | (u8::from(state.valid.velocity) << 3);
    put_u8(buf, 85, flags)?;

    let nav = state.nav.data.unwrap_or_default();
    put_u8(
        buf,
        86,
        match nav.source {
            NavSource::None => 0,
            NavSource::Gps => 1,
            NavSource::Nav1 => 2,
            NavSource::Nav2 => 3,
            NavSource::Unknown => 255,
        },
    )?;
    put_u8(
        buf,
        87,
        match nav.fromto {
            NavFromTo::Off => 0,
            NavFromTo::To => 1,
            NavFromTo::From => 2,
            NavFromTo::Unknown => 255,
        },
    )?;
    put_f32(buf, 88, nav.course_rad)?;
    put_f32(buf, 92, nav.cdi_dots)?;
    put_f32(buf, 96, or_nan(nav.vdev_dots))?;
    put_f32(buf, 100, or_nan(nav.dist_nm))?;
    put_u8(buf, 155, nav.course_reference.to_u8())?;

    put_f32(buf, 104, state.selections.heading_bug_rad)?;
    put_f32(buf, 108, or_nan(state.selections.altitude_sel_m))?;

    let wind = state.wind.data.unwrap_or(Wind {
        from_rad: 0.0,
        speed_mps: 0.0,
    });
    put_f32(buf, 112, wind.from_rad)?;
    put_f32(buf, 116, wind.speed_mps)?;
    put_u32(buf, 120, state.snapshot.generation)?;
    put_u8(
        buf,
        124,
        match state.snapshot.coherence {
            SnapshotCoherence::Insufficient => 0,
            SnapshotCoherence::Coherent => 1,
            SnapshotCoherence::ExcessiveSkew => 2,
            SnapshotCoherence::Unknown => 255,
        },
    )?;
    encode_altitude(state, buf)
}

fn encode_quality(state: &AircraftState, buf: &mut [u8]) -> Result<(), AbiError> {
    put_u8(
        buf,
        84,
        match state.quality {
            EstimateQuality::Good => 0,
            EstimateQuality::Degraded => 1,
            EstimateQuality::Unusable => 2,
            EstimateQuality::Unknown => 255,
        },
    )
}

fn encode_altitude(state: &AircraftState, buf: &mut [u8]) -> Result<(), AbiError> {
    put_u8(buf, 125, state.altitude.reference_class.to_u8())?;
    put_u8(buf, 126, state.selections.altitude_sel_class.to_u8())?;
    put_u8(buf, 127, state.altitude.geoid_model.0)?;
    put_f32(buf, 128, or_nan(state.altitude.sample_m))?;
    put_f32(buf, 132, or_nan(state.selections.baro_sel_hpa))?;
    put_u32(buf, 136, state.altitude.origin.0)?;
    put_u8(buf, 140, state.selections.altitude_sel_model.0)?;
    put_u8(buf, 141, 0)?;
    put_u8(buf, 142, 0)?;
    put_u8(buf, 143, 0)?;
    put_u32(buf, 144, state.selections.altitude_sel_origin.0)?;
    for offset in 148..152 {
        put_u8(buf, offset, 0)?;
    }
    encode_heading(state, buf)
}

mod groups;
use groups::{decode_dynamics, decode_heading, decode_variation, encode_heading};

#[cfg(test)]
mod tests;
