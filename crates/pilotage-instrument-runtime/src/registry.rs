//! The runtime's registry composition and neutral panel helpers.
//!
//! The runtime composes the named [`BUILTIN_SET`] and consumes only the
//! descriptors (ADR-0029, ADR-0032): shells derive their panel map,
//! canvas dimensions, and health keys from this enumeration instead of
//! mirroring constants. The surface is append-only, like every other
//! runtime export.

use indicate_instrument_panels::BUILTIN_SET;
use indicate_instrument_registry::{
    BackgroundCapability, ConfigBlob, DesignFrame, PanelDescriptor, Registry, keys,
};

/// The validated runtime composition, or `None` if the shipped sets no
/// longer compose — which the panels crate's own tests make
/// unreachable. Validated once and memoized: the input is `&'static`
/// and `Registry::from_sets` is pure, so the answer cannot change
/// between calls, and the render path asks for it twice per panel per
/// frame.
pub fn registry() -> Option<Registry> {
    static COMPOSED: std::sync::OnceLock<Option<Registry>> = std::sync::OnceLock::new();
    *COMPOSED.get_or_init(|| Registry::from_sets(&[&BUILTIN_SET]).ok())
}

/// The descriptor at `panel` in composition order, or `None`.
pub fn descriptor(panel: u32) -> Option<&'static PanelDescriptor> {
    registry()?.panels().nth(panel as usize)
}

pub(crate) fn panel_index(id: &str) -> Option<usize> {
    registry()?.panels().position(|panel| panel.id == id)
}

/// The frame this runtime requests for a panel: its first canonical
/// frame, falling back to the floor for a descriptor that declares none.
/// The canonical frames are where the digest, admission matrix, and
/// raster baselines are pinned, so drawing at one keeps every emitted
/// scene comparable with the recorded artefacts. The same value feeds
/// the draw call and the design-dimension helpers, so the frame the
/// producer emits at and the frame the backend maps into pixels cannot
/// diverge.
pub fn canonical_frame(descriptor: &PanelDescriptor) -> DesignFrame {
    descriptor
        .canonical_frames
        .first()
        .copied()
        .unwrap_or(descriptor.frame_min)
}

/// Number of composed panels.
pub fn panel_count() -> u32 {
    registry().map_or(0, |registry| registry.panels().count() as u32)
}

/// Stable panel id (canvas ids and health keys derive from this), or
/// the empty string for an unknown index.
pub fn panel_id(panel: u32) -> String {
    descriptor(panel).map_or_else(String::new, |d| d.id.to_string())
}

/// Operator-facing panel title, or the empty string.
pub fn panel_title(panel: u32) -> String {
    descriptor(panel).map_or_else(String::new, |d| d.title.to_string())
}

/// Required-layer bitset for the panel, or zero.
pub fn panel_required_layers(panel: u32) -> u32 {
    descriptor(panel).map_or(0, |d| u32::from(d.required_layers))
}

/// Required state groups as a bitset over the wire tags, or zero.
pub fn panel_required_groups(panel: u32) -> u32 {
    descriptor(panel).map_or(0, |d| d.required_groups.bits())
}

/// Width of the frame this runtime draws the panel at, in logical
/// units, or zero. This is the same canonical frame the render path
/// hands the panel's draw entry point, so the backend that sizes its
/// canvas from this maps exactly the frame the scene was emitted at.
pub fn panel_design_width(panel: u32) -> f32 {
    descriptor(panel).map_or(0.0, |d| canonical_frame(d).width)
}

/// Height of the frame this runtime draws the panel at, in logical
/// units, or zero.
pub fn panel_design_height(panel: u32) -> f32 {
    descriptor(panel).map_or(0.0, |d| canonical_frame(d).height)
}

/// Background capability code: 0 not-used, 1 opaque, 2 cedeable;
/// 255 for an unknown index (fail-closed, never a default capability).
pub fn background_capability_code(panel: u32) -> u32 {
    descriptor(panel).map_or(255, |d| match d.background {
        BackgroundCapability::NotUsed => 0,
        BackgroundCapability::Opaque => 1,
        BackgroundCapability::Cedeable => 2,
    })
}

/// The scene digest computed by THIS build target over the composed
/// registry and canonical corpus, as lowercase hex. Shells pin it
/// against their own literal (the EXPECTED_GLYPH_SHA256 pattern), so
/// the platform compilation of the panels — not just the host build —
/// must reproduce the one pinned contract.
pub fn scene_digest_hex() -> String {
    let Some(registry) = registry() else {
        return String::new();
    };
    let mut scratch = vec![0u8; indicate_instrument_scene::MAX_SCENE_BYTES];
    match indicate_instrument_registry::scene_digest(&registry, &mut scratch) {
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
pub fn splice_v_speeds(
    blob: &[u8],
    schema: &[indicate_instrument_registry::ConfigKey],
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
