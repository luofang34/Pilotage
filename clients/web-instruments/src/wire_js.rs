//! Shared JS-serialization policy for the wire-decode exports.
//!
//! Every decoded value crosses to JS through [`to_js`], which serializes 64-bit
//! integers as `BigInt` and 32-bit and smaller as `Number`. This gives the
//! browser gates the exact numeric kinds they check for (a `u64` nanosecond
//! stamp is a `BigInt`, a `u32` id is a `Number`), so a wrong-numeric-kind
//! value can never slip through — the fragility the hand-written JS decoder
//! guarded against by hand is now impossible by construction.

use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::JsValue;

/// Serializes `value` to a `JsValue`, mapping 64-bit integers to `BigInt`.
/// Fails closed to `null` if serialization fails, so a caller treats an
/// un-serializable value as an absent decode rather than a partial object.
pub(crate) fn to_js<T: Serialize>(value: &T) -> JsValue {
    let serializer = Serializer::new().serialize_large_number_types_as_bigints(true);
    value.serialize(&serializer).unwrap_or(JsValue::NULL)
}

/// Lowercase hex spelling of a 128-bit incarnation, matching the browser's
/// canonical 32-hex-digit form. The caller guarantees a 16-byte input.
pub(crate) fn incarnation_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// The Latin-1 spelling of a 4-byte codec FourCC, matching the browser's
/// `String.fromCharCode` over the raw bytes.
pub(crate) fn fourcc_string(fourcc: [u8; 4]) -> String {
    fourcc.iter().map(|&b| char::from(b)).collect()
}
