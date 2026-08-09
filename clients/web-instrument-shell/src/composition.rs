//! Wasm screen-composition exports over the runtime's validated
//! composition (ADR-0032). The script builds its cockpit from this
//! enumeration — slot index is paint order — and pins the digest as
//! the fifth compatibility-tuple value. The surface is additive, like
//! every other wasm export here.

use wasm_bindgen::prelude::wasm_bindgen;

/// Number of slots in the shipped screen composition.
#[wasm_bindgen]
pub fn composition_slot_count() -> u32 {
    pilotage_instrument_runtime::composition_slot_count()
}

/// The panel id a slot paints, or the empty string for an unknown slot.
#[wasm_bindgen]
pub fn composition_slot_panel(slot: u32) -> String {
    pilotage_instrument_runtime::composition_slot_panel(slot)
}

/// Left edge of the slot's rectangle in screen units, or zero.
#[wasm_bindgen]
pub fn composition_slot_x(slot: u32) -> f32 {
    pilotage_instrument_runtime::composition_slot_rect(slot).map_or(0.0, |rect| rect.x)
}

/// Top edge of the slot's rectangle in screen units, or zero.
#[wasm_bindgen]
pub fn composition_slot_y(slot: u32) -> f32 {
    pilotage_instrument_runtime::composition_slot_rect(slot).map_or(0.0, |rect| rect.y)
}

/// Width of the slot's rectangle in screen units, or zero.
#[wasm_bindgen]
pub fn composition_slot_width(slot: u32) -> f32 {
    pilotage_instrument_runtime::composition_slot_rect(slot).map_or(0.0, |rect| rect.width)
}

/// Height of the slot's rectangle in screen units, or zero.
#[wasm_bindgen]
pub fn composition_slot_height(slot: u32) -> f32 {
    pilotage_instrument_runtime::composition_slot_rect(slot).map_or(0.0, |rect| rect.height)
}

/// The screen-composition digest computed by THIS build target over the
/// composed registry, as lowercase hex. The script pins it against its
/// own literal (the EXPECTED_SCENE_DIGEST pattern), so the wasm
/// compilation of the layout must reproduce the one pinned contract.
#[wasm_bindgen]
pub fn composition_digest_hex() -> String {
    pilotage_instrument_runtime::composition_digest_hex()
}
