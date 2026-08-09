//! The portable Pilotage instrument runtime (ADR-0032).
//!
//! This crate owns the platform-neutral instrument functions:
//!
//! - state decode and resolve;
//! - feeder state;
//! - alert step;
//! - panel configuration;
//! - scene generation and validation;
//! - successful-production generation;
//! - typed producer status.
//!
//! The crate does not depend on `wasm_bindgen`, `serde_wasm_bindgen`, the
//! Pilotage protocol, JavaScript, Swift, or a UI framework. Shells are
//! thin adapters over this runtime: the browser shell is a thin WASM
//! adapter, the Apple shell is a thin bridge. Both adapters marshal
//! data. Neither adapter owns a decision.
//!
//! Every result is a typed value. A shell packs or marshals the typed
//! values into its own wire format; the packing is the shell's ABI, not
//! this crate's.

pub mod feeder;
mod registry;
mod render_status;
mod runtime;

pub use registry::{
    background_capability_code, canonical_frame, descriptor, panel_count, panel_design_height,
    panel_design_width, panel_id, panel_required_groups, panel_required_layers, panel_title,
    registry, scene_digest_hex, splice_v_speeds,
};
pub use render_status::RenderStatus;
pub use runtime::{
    AlertStepOutcome, RenderOutcome, Runtime, abi_version, derive_alert_events, glyph_manifest,
    glyph_recorded_hash, scene_error_status, scene_format_version,
};

#[cfg(test)]
mod alert_tests;
#[cfg(test)]
mod tests;
