//! Payload codec for the machine-monitoring text group.
//!
//! Fixed 274-byte layout (payload-relative offsets, LE):
//!
//! | off | field |
//! |----:|-------|
//! | 0 | line count u8 |
//! | 1 | reserved, zero |
//! | 2 | revision u32 |
//! | 6 | eight 33-byte line atoms (length u8 + 32 zero-padded bytes) |
//! | 270 | age ms f32 |
//!
//! A fixed frame keeps the codec canonical: unused line slots are
//! all-zero atoms, so equal channels produce equal bytes. Malformed
//! content — an impossible count or an invalid atom — decodes to the
//! fail-closed sentinels [`crate::validate_state`] flags.

use super::{AbiError, get_f32, get_u8, get_u32, put_f32, put_u8, put_u32};
use crate::abi::{opt, or_nan};
use crate::aircraft::{AircraftState, Stamped};
use crate::monitor_text::{MonitorText, TextLine};

const LINE_ATOM: usize = TextLine::CAPACITY + 1;
const LINES_AT: usize = 6;
const AGE_AT: usize = LINES_AT + MonitorText::MAX_LINES * LINE_ATOM;
pub(super) const MONITOR_LEN: usize = AGE_AT + 4;

fn line_at(p: &[u8], index: usize) -> TextLine {
    let off = LINES_AT + index * LINE_ATOM;
    p.get(off..off + LINE_ATOM)
        .and_then(|s| <&[u8; LINE_ATOM]>::try_from(s).ok())
        .map_or(TextLine::INVALID, TextLine::from_wire)
}

pub(super) fn decode_monitor_text(state: &mut AircraftState, p: &[u8]) {
    let age = opt(get_f32(p, AGE_AT));
    state.monitor_text = Stamped {
        data: age.map(|_| {
            let mut lines = [TextLine::EMPTY; MonitorText::MAX_LINES];
            for (index, slot) in lines.iter_mut().enumerate() {
                *slot = line_at(p, index);
            }
            MonitorText::from_wire(get_u32(p, 2), get_u8(p, 0), lines)
        }),
        age_ms: age,
    };
}

pub(super) fn encode_monitor_text(
    state: &AircraftState,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    if state.monitor_text.data.is_none() && state.monitor_text.age_ms.is_none() {
        return Ok(None);
    }
    let p = out.get_mut(..MONITOR_LEN).ok_or(AbiError::Truncated)?;
    let text = state.monitor_text.data.unwrap_or_default();
    put_u8(p, 0, text.line_count());
    put_u8(p, 1, 0);
    put_u32(p, 2, text.revision);
    for (index, line) in text.slots().iter().enumerate() {
        let off = LINES_AT + index * LINE_ATOM;
        if let Some(dst) = p.get_mut(off..off + LINE_ATOM) {
            dst.copy_from_slice(&line.to_wire());
        }
    }
    put_f32(p, AGE_AT, or_nan(state.monitor_text.age_ms));
    Ok(Some(MONITOR_LEN))
}
