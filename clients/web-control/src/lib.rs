//! The browser viewer's wasm-bindgen surface over the portable control
//! runtime (ADR-0007). The browser owns only Gamepad API sampling, DOM,
//! WebTransport, and the execution of returned actions; every mapping and
//! authority decision is `pilotage-control`'s, re-exported here so the
//! browser and native clients run the same code.

pub use pilotage_control::*;

#[cfg(target_arch = "wasm32")]
mod wasm;
