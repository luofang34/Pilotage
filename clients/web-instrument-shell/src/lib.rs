//! Thin WASM adapter over the Pilotage instrument runtime (ADR-0032).
//!
//! The `pilotage-instrument-runtime` crate owns the instrument logic.
//! This adapter owns the wasm ABI: it marshals values, packs typed
//! outcomes, and holds the `wasm_bindgen` attributes. It does not own a
//! decision.
//!
//! wasm-bindgen exposes an explicit [`InstrumentRuntime`] resource so each JS
//! owner has independent buffers, configuration, and generations without
//! module-level mutable state:
//!
//! 1. JS constructs [`InstrumentRuntime`], calls [`InstrumentRuntime::init`],
//!    then queries its fixed state and scene buffer offsets.
//! 2. Each frame, JS writes a packed
//!    [`indicate_instrument_state::abi`] state block into the state
//!    buffer and calls [`InstrumentRuntime::render_result`] with a panel id.
//! 3. The returned `u64` carries status in bits 0..7, scene length in
//!    bits 8..31, and generation in bits 32..63. Status zero means the scene
//!    was drawn and structurally self-validated; any failure carries a zero
//!    length and the scene buffer must not be painted.
//!
//! Buffers are allocated once and never grow, so the pointers stay valid
//! until explicit reinitialization. The packed generation advances only on
//! success, giving consumers a liveness signal that cannot be faked by failed
//! attempts. Successful scene bytes remain valid until the next render attempt
//! or reinitialization and must be consumed within that interval.

mod compatibility;
mod composition;
mod exports;
mod feeder_exports;
mod panel_registry;

pub use exports::{InstrumentRuntime, abi_version};
pub use pilotage_instrument_runtime::RenderStatus;

#[cfg(test)]
mod tests;
