//! WASM exports for the Indicate compatibility identity.

use wasm_bindgen::prelude::wasm_bindgen;

/// Scene format version linked into this WASM module.
#[wasm_bindgen]
pub fn scene_format_version() -> u32 {
    pilotage_instrument_runtime::scene_format_version()
}

/// Conformance-corpus version linked into this WASM module.
#[wasm_bindgen]
pub fn corpus_version() -> u32 {
    pilotage_instrument_runtime::corpus_version()
}

/// Conformance-corpus digest linked into this WASM module.
#[wasm_bindgen]
pub fn corpus_digest_hex() -> String {
    pilotage_instrument_runtime::corpus_digest_hex().to_string()
}
