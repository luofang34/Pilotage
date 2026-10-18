//! The MAVLink parameter protocol subset: PARAM_SET out, PARAM_VALUE in.
//!
//! A flight controller answers every PARAM_SET with a PARAM_VALUE that
//! carries the value it now holds, so the echo is the only confirmation a
//! write took effect. INT32 parameters travel bytewise in the `f32` wire
//! field (the PX4 convention): the integer's bits, not its numeric value.

use super::encoding::{encode_frame_v2, encode_frame_v2_from};
use super::{FcMessage, PARAM_SET_ID, PARAM_VALUE_ID};

/// MAV_PARAM_TYPE_INT32.
const PARAM_TYPE_INT32: u8 = 6;
/// MAV_PARAM_TYPE_REAL32.
const PARAM_TYPE_REAL32: u8 = 9;
/// The fixed width of a parameter name on the wire, NUL-padded.
pub const PARAM_ID_LEN: usize = 16;

/// One typed parameter value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    /// A 32-bit signed integer parameter.
    Int32(i32),
    /// A 32-bit float parameter.
    Real32(f32),
}

impl ParamValue {
    fn wire(self) -> ([u8; 4], u8) {
        match self {
            Self::Int32(value) => (value.to_le_bytes(), PARAM_TYPE_INT32),
            Self::Real32(value) => (value.to_le_bytes(), PARAM_TYPE_REAL32),
        }
    }

    fn from_wire(bytes: [u8; 4], param_type: u8) -> Option<Self> {
        match param_type {
            PARAM_TYPE_INT32 => Some(Self::Int32(i32::from_le_bytes(bytes))),
            PARAM_TYPE_REAL32 => Some(Self::Real32(f32::from_le_bytes(bytes))),
            _ => None,
        }
    }
}

/// A parameter name as the wire carries it: up to 16 ASCII bytes,
/// NUL-padded. Longer names are truncated, as the protocol allows no more.
#[must_use]
pub fn param_id(name: &str) -> [u8; PARAM_ID_LEN] {
    let mut id = [0u8; PARAM_ID_LEN];
    for (slot, byte) in id.iter_mut().zip(name.bytes()) {
        *slot = byte;
    }
    id
}

/// Serializes a PARAM_SET for the selected MAVLink system and component.
///
/// Wire order (23 bytes): param_value @0, target_system @4,
/// target_component @5, param_id\[16\] @6, param_type @22.
#[must_use]
pub fn encode_param_set(
    seq: u8,
    name: &str,
    value: ParamValue,
    target_system: u8,
    target_component: u8,
) -> [u8; 35] {
    let (bytes, param_type) = value.wire();
    let mut payload = [0u8; 23];
    payload[0..4].copy_from_slice(&bytes);
    payload[4] = target_system;
    payload[5] = target_component;
    payload[6..22].copy_from_slice(&param_id(name));
    payload[22] = param_type;
    let mut frame = [0u8; 35];
    encode_frame_v2(seq, PARAM_SET_ID, &payload, 168, &mut frame);
    frame
}

/// PARAM_VALUE wire order (25 bytes): param_value @0, param_count u16 @4,
/// param_index u16 @6, param_id\[16\] @8, param_type @24. A type other than
/// INT32 or REAL32 is not decoded.
pub(super) fn decode_param_value(payload: &[u8]) -> Option<FcMessage> {
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = payload.get(index).copied().unwrap_or(0);
    }
    let mut id = [0u8; PARAM_ID_LEN];
    for (index, slot) in id.iter_mut().enumerate() {
        *slot = payload.get(8 + index).copied().unwrap_or(0);
    }
    let value = ParamValue::from_wire(bytes, payload.get(24).copied().unwrap_or(0))?;
    Some(FcMessage::ParamValue {
        param_id: id,
        value,
    })
}

/// Serializes an FC-sourced PARAM_VALUE (index and count zero): the echo
/// an autopilot returns for a PARAM_SET, for fake FCs in tests.
#[must_use]
pub fn encode_param_value(
    seq: u8,
    name: &str,
    value: ParamValue,
    system_id: u8,
    component_id: u8,
) -> [u8; 37] {
    let (bytes, param_type) = value.wire();
    let mut payload = [0u8; 25];
    payload[0..4].copy_from_slice(&bytes);
    payload[8..24].copy_from_slice(&param_id(name));
    payload[24] = param_type;
    let mut frame = [0u8; 37];
    encode_frame_v2_from(
        seq,
        PARAM_VALUE_ID,
        &payload,
        220,
        system_id,
        component_id,
        &mut frame,
    );
    frame
}
