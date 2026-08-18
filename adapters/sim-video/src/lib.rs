//! Simulator-neutral camera sidecar client and video frame identity.
//!
//! A simulator video producer (the C++ gz-transport sidecar, an X-Plane
//! window-capture sidecar, or any future engine's) speaks one protocol:
//! length-delimited `pilotage.bridge.v1` envelopes over a localhost TCP
//! connection it dials back into. This crate owns that client side —
//! spawn, framing, decode, bounded frame delivery — plus the capture
//! identity stamping (ADR-0020) every adapter applies before a frame
//! reaches the host media plane. Adapters stay engine-specific; the
//! video plumbing does not.
//!
//! This crate is I/O-bearing (`adapters/` is exempt from the sans-IO
//! rule, ADR-0002); no wire type here crosses into `pilotage-protocol`.

mod bridge_client;
mod error;
mod frame;
mod framing;
mod video;
pub mod wire;

pub use bridge_client::{BRIDGE_BIN_ENV, BridgeClient, BridgeConfig, LatestBridgeState};
pub use error::SimVideoError;
pub use frame::{
    CHASE_CAMERA, CHASE_SOURCE_ID, FPV_CAMERA, FPV_SOURCE_ID, GIMBAL_CAMERA, GIMBAL_SOURCE_ID,
    RawVideoFrame,
};
pub use framing::read_envelope;
pub use video::FrameStamper;
