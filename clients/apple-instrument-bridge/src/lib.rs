//! The thin Apple bridge to the Pilotage instrument runtime (ADR-0032).
//!
//! This crate is a narrow generated FFI surface. uniffi generates the
//! bindings from the exported items. A hand-written C header was
//! rejected by ADR-0032.
//!
//! The ownership split:
//!
//! - Indicate owns the instrument contract and the panels.
//! - `pilotage-instrument-runtime` owns the instrument logic.
//! - IndicateAppleDisplay owns display interpretation and failure
//!   latching on the Apple platform.
//! - This bridge marshals data. It holds no panel logic and no state
//!   derivation. It holds no latching and no interpretation helper.
//!
//! The composition call steps alerts once and produces all panel scenes.
//! The bridge does not export a per-panel render call.
//!
//! Companion-computer builds do not link this crate. The crate has no
//! Apple-platform dependency: it builds on any host, and CI proves it.
//!
//! The digest functions are the consumer's before-paint compatibility
//! check input (ADR-0032's tuple). The caller verifies them against its
//! pinned values before first paint. The caller does not paint on
//! mismatch. The verification is the caller's gate, not this crate's.

uniffi::setup_scaffolding!();

mod bridge;
mod enumeration;
mod records;

pub use bridge::InstrumentBridge;
pub use enumeration::{
    composition_digest_hex, composition_slot, composition_slot_count, corpus_digest_hex,
    corpus_version, glyph_asset, panel_count, panel_descriptor, scene_digest_hex,
    scene_format_version, state_abi_version,
};
pub use records::{
    BridgeCompositionFrameOutcome, BridgeCompositionPanelOutcome, BridgeCompositionSlot,
    BridgeGlyphAsset, BridgePanelDescriptor, BridgeWriteOutcome,
};

#[cfg(test)]
mod tests;
