//! The shell's registry composition and panel-enumeration surface.
//!
//! The browser shell composes [`BUILTIN_PANELS`] and consumes only the
//! descriptors (ADR-0029): the script derives its panel map, canvas
//! dimensions, and health keys from this enumeration instead of
//! mirroring shell constants. The surface is append-only, like every
//! other wasm export here.

use pilotage_instrument_panels::BUILTIN_PANELS;
use pilotage_instrument_registry::{
    BackgroundCapability, ConfigBlob, PanelDescriptor, Registry, keys,
};
use wasm_bindgen::prelude::wasm_bindgen;

/// The validated shell composition, or `None` if the shipped panels no
/// longer compose — which the panels crate's own tests make unreachable.
pub(crate) fn registry() -> Option<Registry> {
    Registry::new(BUILTIN_PANELS).ok()
}

pub(crate) fn descriptor(panel: u32) -> Option<&'static PanelDescriptor> {
    registry()?.panels().get(panel as usize)
}

/// Number of composed panels.
#[wasm_bindgen]
pub fn panel_count() -> u32 {
    registry().map_or(0, |registry| registry.panels().len() as u32)
}

/// Stable panel id (canvas ids and health keys derive from this), or
/// the empty string for an unknown index.
#[wasm_bindgen]
pub fn panel_id(panel: u32) -> String {
    descriptor(panel).map_or_else(String::new, |d| d.id.to_string())
}

/// Operator-facing panel title, or the empty string.
#[wasm_bindgen]
pub fn panel_title(panel: u32) -> String {
    descriptor(panel).map_or_else(String::new, |d| d.title.to_string())
}

/// Required-layer bitset for the panel, or zero.
#[wasm_bindgen]
pub fn panel_required_layers(panel: u32) -> u32 {
    descriptor(panel).map_or(0, |d| u32::from(d.required_layers))
}

/// Required state groups as a bitset over the wire tags, or zero.
#[wasm_bindgen]
pub fn panel_required_groups(panel: u32) -> u32 {
    descriptor(panel).map_or(0, |d| d.required_groups.bits())
}

/// Design-frame width in logical units, or zero.
#[wasm_bindgen]
pub fn panel_design_width(panel: u32) -> f32 {
    descriptor(panel).map_or(0.0, |d| d.design_frame.width)
}

/// Design-frame height in logical units, or zero.
#[wasm_bindgen]
pub fn panel_design_height(panel: u32) -> f32 {
    descriptor(panel).map_or(0.0, |d| d.design_frame.height)
}

/// Background capability code: 0 not-used, 1 opaque, 2 cedeable;
/// 255 for an unknown index (fail-closed, never a default capability).
#[wasm_bindgen]
pub fn panel_background_capability(panel: u32) -> u32 {
    descriptor(panel).map_or(255, |d| match d.background {
        BackgroundCapability::NotUsed => 0,
        BackgroundCapability::Opaque => 1,
        BackgroundCapability::Cedeable => 2,
    })
}

/// The scene digest computed by THIS build target over the composed
/// registry and canonical corpus, as lowercase hex. The script pins it
/// against its own literal (the EXPECTED_GLYPH_SHA256 pattern), so the
/// wasm compilation of the panels — not just the host build — must
/// reproduce the one pinned contract.
#[wasm_bindgen]
pub fn scene_digest_hex() -> String {
    let Some(registry) = registry() else {
        return String::new();
    };
    let mut scratch = vec![0u8; pilotage_instrument_scene::MAX_SCENE_BYTES];
    match pilotage_instrument_registry::scene_digest(&registry, &mut scratch) {
        Ok(digest) => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut out = String::with_capacity(64);
            for byte in digest {
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
            out
        }
        // An empty string matches no pin: a digest failure fails visibly.
        Err(_) => String::new(),
    }
}

/// Replaces the V_SPEEDS entry inside `blob` (encoded config TLV),
/// preserving every other entry and ascending key order. `payload`
/// `None` removes the entry. Iterates the panel's own schema — which
/// [`Registry::new`] proves strictly ascending — so a schema that
/// grows a key can never be silently dropped by this splice.
pub(crate) fn splice_v_speeds(
    blob: &[u8],
    schema: &[pilotage_instrument_registry::ConfigKey],
    payload: Option<[u8; 20]>,
) -> Option<Vec<u8>> {
    let parsed = ConfigBlob::parse(blob).ok()?;
    let mut out = Vec::new();
    for key in schema {
        if *key == keys::V_SPEEDS {
            if let Some(payload) = payload {
                push_entry(&mut out, key.0, &payload);
            }
        } else if let Some(value) = parsed.get(*key) {
            push_entry(&mut out, key.0, value);
        }
    }
    Some(out)
}

fn push_entry(out: &mut Vec<u8>, key: u16, payload: &[u8]) {
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
}

#[cfg(test)]
mod tests;
