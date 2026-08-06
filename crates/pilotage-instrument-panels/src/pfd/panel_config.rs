//! PFD configuration decoding from the shell-delivered blob (ADR-0033).

use pilotage_instrument_registry::{ConfigBlob, ConfigError, ConfigKey, keys};

use super::{BackgroundMode, PfdConfig, SvsViewport, VSpeeds};
use crate::{PANEL_H, PANEL_W};

/// Every key the PFD understands; a shell refuses anything else before
/// the blob reaches this decoder, and the decoder refuses it again.
pub const PFD_CONFIG_SCHEMA: &[ConfigKey] = &[
    keys::BACKGROUND_MODE,
    keys::V_SPEEDS,
    keys::SVS_VIEWPORT,
    keys::SVS_QUALITY,
];

impl PfdConfig {
    /// Decodes `config`; absent keys keep the shipped defaults, and an
    /// uninterpretable value is refused whole — a config error must
    /// never half-apply. Every present key is validated regardless of
    /// which mode consumes it, and an SVS key set under a non-SVS
    /// background is refused as inert: silently ignoring an option a
    /// caller believes is set would misstate what the panel displays
    /// (ADR-0033).
    pub fn from_config(config: &ConfigBlob<'_>) -> Result<PfdConfig, ConfigError> {
        config.require_schema(PFD_CONFIG_SCHEMA)?;
        let viewport = decode_viewport(config)?;
        let quality = decode_quality(config)?;
        Ok(PfdConfig {
            background: decode_background(config, viewport, quality)?,
            v_speeds: decode_v_speeds(config)?,
        })
    }
}

fn decode_background(
    config: &ConfigBlob<'_>,
    viewport: Option<SvsViewport>,
    quality: Option<u8>,
) -> Result<BackgroundMode, ConfigError> {
    let mode = config.get(keys::BACKGROUND_MODE);
    if !matches!(mode, Some([2])) {
        if viewport.is_some() {
            return Err(ConfigError::InertKey {
                key: keys::SVS_VIEWPORT.0,
            });
        }
        if quality.is_some() {
            return Err(ConfigError::InertKey {
                key: keys::SVS_QUALITY.0,
            });
        }
    }
    match mode {
        None | Some([0]) => Ok(BackgroundMode::Horizon),
        Some([1]) => Ok(BackgroundMode::None),
        Some([2]) => Ok(BackgroundMode::Svs {
            viewport: viewport.unwrap_or(SvsViewport {
                x: 0.0,
                y: 0.0,
                width: PANEL_W,
                height: PANEL_H,
            }),
            quality: quality.unwrap_or(0),
        }),
        Some(other) => Err(ConfigError::BadValue {
            key: keys::BACKGROUND_MODE.0,
            len: other.len(),
        }),
    }
}

fn decode_viewport(config: &ConfigBlob<'_>) -> Result<Option<SvsViewport>, ConfigError> {
    let Some(bytes) = config.get(keys::SVS_VIEWPORT) else {
        return Ok(None);
    };
    let bad = || ConfigError::BadValue {
        key: keys::SVS_VIEWPORT.0,
        len: bytes.len(),
    };
    let [x, y, width, height] = decode_f32s::<4>(bytes).ok_or_else(bad)?;
    // Within the design frame, as the descriptor field documents —
    // imagery cannot be requested where the panel does not draw.
    let sane = x >= 0.0
        && y >= 0.0
        && width > 0.0
        && height > 0.0
        && x + width <= PANEL_W
        && y + height <= PANEL_H;
    if sane {
        Ok(Some(SvsViewport {
            x,
            y,
            width,
            height,
        }))
    } else {
        Err(bad())
    }
}

fn decode_quality(config: &ConfigBlob<'_>) -> Result<Option<u8>, ConfigError> {
    match config.get(keys::SVS_QUALITY) {
        None => Ok(None),
        Some([quality]) => Ok(Some(*quality)),
        Some(other) => Err(ConfigError::BadValue {
            key: keys::SVS_QUALITY.0,
            len: other.len(),
        }),
    }
}

fn decode_v_speeds(config: &ConfigBlob<'_>) -> Result<Option<VSpeeds>, ConfigError> {
    let Some(bytes) = config.get(keys::V_SPEEDS) else {
        return Ok(None);
    };
    let bad = || ConfigError::BadValue {
        key: keys::V_SPEEDS.0,
        len: bytes.len(),
    };
    let [vs0, vs, vfe, vno, vne] = decode_f32s::<5>(bytes).ok_or_else(bad)?;
    let speeds = [vs0, vs, vfe, vno, vne];
    // The tape draws bands only over a coherent ladder; a non-finite,
    // unordered, or collapsed set (which would paint a nearly all-red
    // tape) is refused rather than painted somewhere misleading. The
    // band-defining speeds must be strictly ordered.
    let sane = speeds.iter().all(|v| v.is_finite() && *v >= 0.0)
        && vs0 <= vs
        && vs < vfe
        && vfe <= vno
        && vno < vne;
    if sane {
        Ok(Some(VSpeeds {
            vs0_kt: vs0,
            vs_kt: vs,
            vfe_kt: vfe,
            vno_kt: vno,
            vne_kt: vne,
        }))
    } else {
        Err(bad())
    }
}

fn decode_f32s<const N: usize>(bytes: &[u8]) -> Option<[f32; N]> {
    if bytes.len() != N * 4 {
        return None;
    }
    let mut out = [0.0f32; N];
    for (i, slot) in out.iter_mut().enumerate() {
        let word: [u8; 4] = bytes.get(i * 4..i * 4 + 4)?.try_into().ok()?;
        *slot = f32::from_le_bytes(word);
    }
    Some(out)
}
