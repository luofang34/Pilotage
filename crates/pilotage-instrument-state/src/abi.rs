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
//! | 0   | u32  | version (=1) |
//! | 4   | f32×4| attitude quaternion w, x, y, z |
//! | 20  | f32×3| body rates p, q, r (rad/s) |
//! | 32  | f32×3| position NED n, e, d (m) |
//! | 44  | f32×3| velocity NED n, e, d (m/s) |
//! | 56  | f32  | IAS (m/s, NaN absent) |
//! | 60  | f32  | baro setting (hPa, NaN absent) |
//! | 64  | f32×5| ages ms: attitude, kinematics, air, nav, wind (NaN never) |
//! | 84  | u8   | quality (0 good, 1 degraded, 2 unusable) |
//! | 85  | u8   | valid flags (bit0 att, 1 rates, 2 pos, 3 vel) |
//! | 86  | u8   | nav source (0 none, 1 gps, 2 nav1, 3 nav2) |
//! | 87  | u8   | nav from/to (0 off, 1 to, 2 from) |
//! | 88  | f32  | nav course (rad) |
//! | 92  | f32  | nav lateral deviation (dots) |
//! | 96  | f32  | nav vertical deviation (dots, NaN absent) |
//! | 100 | f32  | nav distance (NM, NaN absent) |
//! | 104 | f32  | heading bug (rad) |
//! | 108 | f32  | selected altitude (m, NaN none) |
//! | 112 | f32  | wind from (rad) |
//! | 116 | f32  | wind speed (m/s) |

use crate::aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, Stamped, ValidFlags, Wind,
};
use crate::quat::Quat;

/// Version stamped in the block's first four bytes.
pub const STATE_ABI_VERSION: u32 = 1;

/// Size of the packed block in bytes.
pub const STATE_ABI_SIZE: usize = 120;

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

fn f32_at(buf: &[u8], off: usize) -> Result<f32, AbiError> {
    let b = buf.get(off..off + 4).ok_or(AbiError::Truncated)?;
    Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u8_at(buf: &[u8], off: usize) -> Result<u8, AbiError> {
    buf.get(off).copied().ok_or(AbiError::Truncated)
}

fn opt(v: f32) -> Option<f32> {
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

    let source = match u8_at(buf, 86)? {
        1 => NavSource::Gps,
        2 => NavSource::Nav1,
        3 => NavSource::Nav2,
        _ => NavSource::None,
    };
    let fromto = match u8_at(buf, 87)? {
        1 => NavFromTo::To,
        2 => NavFromTo::From,
        _ => NavFromTo::Off,
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

    let quality = match u8_at(buf, 84)? {
        1 => EstimateQuality::Degraded,
        2 => EstimateQuality::Unusable,
        _ => EstimateQuality::Good,
    };
    let flags = u8_at(buf, 85)?;
    let valid = ValidFlags {
        attitude: flags & 0b0001 != 0,
        rates: flags & 0b0010 != 0,
        position: flags & 0b0100 != 0,
        velocity: flags & 0b1000 != 0,
    };

    Ok(AircraftState {
        attitude,
        kinematics,
        air,
        nav,
        wind,
        selections: Selections {
            heading_bug_rad: f32_at(buf, 104)?,
            altitude_sel_m: opt(f32_at(buf, 108)?),
        },
        quality,
        valid,
    })
}

fn put_f32(buf: &mut [u8], off: usize, v: f32) -> Result<(), AbiError> {
    let b = buf.get_mut(off..off + 4).ok_or(AbiError::Truncated)?;
    b.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn put_u8(buf: &mut [u8], off: usize, v: u8) -> Result<(), AbiError> {
    *buf.get_mut(off).ok_or(AbiError::Truncated)? = v;
    Ok(())
}

fn or_nan(v: Option<f32>) -> f32 {
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

    put_u8(
        buf,
        84,
        match state.quality {
            EstimateQuality::Good => 0,
            EstimateQuality::Degraded => 1,
            EstimateQuality::Unusable => 2,
        },
    )?;
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
        },
    )?;
    put_u8(
        buf,
        87,
        match nav.fromto {
            NavFromTo::Off => 0,
            NavFromTo::To => 1,
            NavFromTo::From => 2,
        },
    )?;
    put_f32(buf, 88, nav.course_rad)?;
    put_f32(buf, 92, nav.cdi_dots)?;
    put_f32(buf, 96, or_nan(nav.vdev_dots))?;
    put_f32(buf, 100, or_nan(nav.dist_nm))?;

    put_f32(buf, 104, state.selections.heading_bug_rad)?;
    put_f32(buf, 108, or_nan(state.selections.altitude_sel_m))?;

    let wind = state.wind.data.unwrap_or(Wind {
        from_rad: 0.0,
        speed_mps: 0.0,
    });
    put_f32(buf, 112, wind.from_rad)?;
    put_f32(buf, 116, wind.speed_mps)?;
    Ok(())
}

#[cfg(test)]
mod tests;
