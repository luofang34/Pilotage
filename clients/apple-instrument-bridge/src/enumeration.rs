//! The enumeration and digest-identity functions: panel descriptors,
//! the screen-composition layout, and the compatibility-tuple values.
//! Each function mirrors one neutral helper of the runtime and adds no
//! judgement of its own.

use pilotage_instrument_runtime::{canonical_frame, descriptor};

use crate::records::{BridgeCompositionSlot, BridgePanelDescriptor};

/// Number of panels in the composed registry.
#[uniffi::export]
pub fn panel_count() -> u32 {
    pilotage_instrument_runtime::panel_count()
}

/// The descriptor at `index` in composition order, or `None` for an
/// unknown index.
#[uniffi::export]
pub fn panel_descriptor(index: u32) -> Option<BridgePanelDescriptor> {
    let descriptor = descriptor(index)?;
    let frame = canonical_frame(descriptor);
    Some(BridgePanelDescriptor {
        id: descriptor.id.to_string(),
        title: descriptor.title.to_string(),
        required_layers: u32::from(descriptor.required_layers),
        required_groups: descriptor.required_groups.bits(),
        design_width: frame.width,
        design_height: frame.height,
        background_capability: pilotage_instrument_runtime::background_capability_code(index),
    })
}

/// Number of slots in the validated screen composition.
#[uniffi::export]
pub fn composition_slot_count() -> u32 {
    pilotage_instrument_runtime::composition_slot_count()
}

/// The slot at `index` in paint order, or `None` for an unknown index.
#[uniffi::export]
pub fn composition_slot(index: u32) -> Option<BridgeCompositionSlot> {
    let rect = pilotage_instrument_runtime::composition_slot_rect(index)?;
    Some(BridgeCompositionSlot {
        panel: pilotage_instrument_runtime::composition_slot_panel(index),
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    })
}

/// The state-frame ABI version this build was compiled against:
/// compatibility-tuple value 1.
#[uniffi::export]
pub fn state_abi_version() -> u32 {
    pilotage_instrument_runtime::abi_version()
}

/// The scene format version this build was compiled against:
/// compatibility-tuple value 2.
#[uniffi::export]
pub fn scene_format_version() -> u32 {
    pilotage_instrument_runtime::scene_format_version()
}

/// The conformance-corpus version this build was compiled against:
/// compatibility-tuple value 3.
#[uniffi::export]
pub fn corpus_version() -> u32 {
    pilotage_instrument_runtime::corpus_version()
}

/// The conformance-corpus digest as lowercase hex: compatibility-tuple
/// value 3.
#[uniffi::export]
pub fn corpus_digest_hex() -> String {
    pilotage_instrument_runtime::corpus_digest_hex().to_string()
}

/// The registry scene digest as lowercase hex: compatibility-tuple
/// value 4.
#[uniffi::export]
pub fn scene_digest_hex() -> String {
    pilotage_instrument_runtime::scene_digest_hex()
}

/// The screen-composition digest as lowercase hex: compatibility-tuple
/// value 5.
#[uniffi::export]
pub fn composition_digest_hex() -> String {
    pilotage_instrument_runtime::composition_digest_hex()
}
