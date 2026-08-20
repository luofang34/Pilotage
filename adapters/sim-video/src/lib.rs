//! Simulator video sidecar client: one protocol for every producer.
//!
//! A simulator video producer (the C++ gz-transport sidecar, an X-Plane
//! window-capture sidecar, or any future engine's) speaks one protocol:
//! length-delimited `pilotage.bridge.v1` envelopes over a localhost TCP
//! connection it dials back into. This crate owns that client side —
//! spawn, framing, decode, bounded frame delivery — and the conversion
//! to the engine-neutral `pilotage-adapter-api` frame types. It never
//! reaches a flight build: producers exist only where a simulator does.
//!
//! This crate is I/O-bearing (`adapters/` is exempt from the sans-IO
//! rule, ADR-0002); no wire type here crosses into `pilotage-protocol`.

mod bridge_client;
mod convert;
mod error;
mod framing;
mod pump;
pub mod wire;

pub use bridge_client::{BRIDGE_BIN_ENV, BridgeClient, BridgeConfig, LatestBridgeState};
pub use error::SimVideoError;
pub use framing::read_envelope;
