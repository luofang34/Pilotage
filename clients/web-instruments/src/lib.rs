//! WASM export of the instrument panels (ADR-0017's first backend).
//!
//! The ABI is deliberately primitive — no wasm-bindgen, no glue codegen:
//!
//! 1. JS calls [`init`] once, then [`state_ptr`]/[`scene_ptr`] for the
//!    fixed buffer locations in linear memory.
//! 2. Each frame, JS writes a packed
//!    [`pilotage_instrument_state::abi`] state block into the state
//!    buffer and calls [`render_status()`] with a panel id.
//! 3. A `0` status means the scene was drawn and structurally
//!    self-validated: JS reads [`scene_len()`] bytes from the scene
//!    buffer and paints them. Any other status is a stable
//!    [`RenderStatus`] reason code and the scene buffer must not be
//!    painted (DISP-01: failures are visible, never a stale frame).
//!
//! Buffers are allocated once and never grow, so the pointers stay valid
//! for the life of the instance. [`render_generation()`] advances only
//! on success, giving consumers a liveness signal that cannot be faked
//! by failed attempts.

mod exports;
mod render_status;

pub use exports::{
    abi_version, init, render_generation, render_status, scene_len, scene_ptr, set_v_speeds,
    state_len, state_ptr,
};
pub use render_status::RenderStatus;

#[cfg(test)]
mod tests;
