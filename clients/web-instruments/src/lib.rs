//! WASM export of the instrument panels (ADR-0017's first backend).
//!
//! wasm-bindgen exposes an explicit [`InstrumentRuntime`] resource so each JS
//! owner has independent buffers, configuration, and generations without
//! module-level mutable state:
//!
//! 1. JS constructs [`InstrumentRuntime`], calls [`InstrumentRuntime::init`],
//!    then queries its fixed state and scene buffer offsets.
//! 2. Each frame, JS writes a packed
//!    [`pilotage_instrument_state::abi`] state block into the state
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

mod classify_h264;
mod decode_envelope;
mod decode_video;
mod exports;
mod render_status;
mod wire_js;

pub use exports::{InstrumentRuntime, abi_version};
pub use render_status::RenderStatus;

#[cfg(test)]
mod alert_tests;
#[cfg(test)]
mod tests;
