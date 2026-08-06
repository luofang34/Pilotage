//! The browser wasm bundle: the instrument shell plus the wire/video
//! decode exports, in one cdylib so the viewer loads one module.
//!
//! The instrument half lives in `pilotage-instruments-web-shell`, which
//! has no protocol dependency by construction — the split is the
//! ADR-0034 cut line, enforced by each crate's dependency set rather than by
//! convention. The re-export below is load-bearing: it links the shell
//! rlib into this cdylib so its `wasm_bindgen` exports survive.

mod classify_h264;
mod decode_envelope;
mod decode_video;
mod wire_js;

pub use pilotage_instruments_web_shell::{InstrumentRuntime, RenderStatus, abi_version};
