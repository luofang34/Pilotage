//! Wire-level identifiers and control-frame vocabulary for Pilotage.
//!
//! Types generated from `schemas/` live in [`wire`] alongside the
//! hand-written identifier and control types below, per ADR-0002 and
//! ADR-0014. The crate-private `convert` module bridges the two.

mod control;
mod convert;
mod ids;
pub mod wire;

pub use control::{ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame};
pub use convert::{
    ConvertError, DecodeError, SCHEMA_VERSION, decode_control_frame_envelope,
    decode_envelope_length_delimited, encode_control_frame_envelope,
    encode_envelope_length_delimited,
};
pub use ids::{Generation, PrincipalId, ScopeId, SequenceNum, SessionId, VehicleId};
