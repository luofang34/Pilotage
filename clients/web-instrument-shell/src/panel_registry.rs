//! Wasm panel-enumeration exports over the runtime's registry helpers.
//!
//! The browser shell composes nothing of its own: the script derives its
//! panel map, canvas dimensions, and health keys from this enumeration
//! (ADR-0029), and every answer comes from the portable runtime
//! (ADR-0032). The surface is append-only, like every other wasm export
//! here.

use wasm_bindgen::prelude::wasm_bindgen;

/// Number of composed panels.
#[wasm_bindgen]
pub fn panel_count() -> u32 {
    pilotage_instrument_runtime::panel_count()
}

/// Stable panel id (canvas ids and health keys derive from this), or
/// the empty string for an unknown index.
#[wasm_bindgen]
pub fn panel_id(panel: u32) -> String {
    pilotage_instrument_runtime::panel_id(panel)
}

/// Operator-facing panel title, or the empty string.
#[wasm_bindgen]
pub fn panel_title(panel: u32) -> String {
    pilotage_instrument_runtime::panel_title(panel)
}

/// Required-layer bitset for the panel, or zero.
#[wasm_bindgen]
pub fn panel_required_layers(panel: u32) -> u32 {
    pilotage_instrument_runtime::panel_required_layers(panel)
}

/// Required state groups as a bitset over the wire tags, or zero.
#[wasm_bindgen]
pub fn panel_required_groups(panel: u32) -> u32 {
    pilotage_instrument_runtime::panel_required_groups(panel)
}

/// Width of the frame this shell draws the panel at, in logical units,
/// or zero. This is the same canonical frame the render path hands the
/// panel's draw entry point, so the backend that sizes its canvas from
/// this maps exactly the frame the scene was emitted at.
#[wasm_bindgen]
pub fn panel_design_width(panel: u32) -> f32 {
    pilotage_instrument_runtime::panel_design_width(panel)
}

/// Height of the frame this shell draws the panel at, in logical units,
/// or zero.
#[wasm_bindgen]
pub fn panel_design_height(panel: u32) -> f32 {
    pilotage_instrument_runtime::panel_design_height(panel)
}

/// Background capability code: 0 not-used, 1 opaque, 2 cedeable;
/// 255 for an unknown index (fail-closed, never a default capability).
#[wasm_bindgen]
pub fn panel_background_capability(panel: u32) -> u32 {
    pilotage_instrument_runtime::background_capability_code(panel)
}

/// The scene digest computed by THIS build target over the composed
/// registry and canonical corpus, as lowercase hex. The script pins it
/// against its own literal (the EXPECTED_GLYPH_SHA256 pattern), so the
/// wasm compilation of the panels — not just the host build — must
/// reproduce the one pinned contract.
#[wasm_bindgen]
pub fn scene_digest_hex() -> String {
    pilotage_instrument_runtime::scene_digest_hex()
}
