//! WASM export of the instrument panels (ADR-0017's first backend).
//!
//! The ABI is deliberately primitive — no wasm-bindgen, no glue codegen:
//!
//! 1. JS calls [`init`] once, then [`state_ptr`]/[`scene_ptr`] for the
//!    fixed buffer locations in linear memory.
//! 2. Each frame, JS writes a packed
//!    [`pilotage_instrument_state::abi`] state block into the state
//!    buffer and calls [`render`] with a panel id.
//! 3. `render` decodes the state, resolves it, draws the panel, and
//!    returns the encoded scene length; JS interprets the scene bytes
//!    onto a Canvas2D.
//!
//! Buffers are allocated once and never grow, so the pointers stay valid
//! for the life of the instance.

mod exports;

pub use exports::{abi_version, init, render, scene_ptr, set_v_speeds, state_len, state_ptr};

#[cfg(test)]
mod tests;
