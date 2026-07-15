//! Wire-level identifiers and control-frame vocabulary for Pilotage.
//!
//! Types generated from `schemas/` live in [`wire`] alongside the
//! hand-written identifier and control types below, per ADR-0002 and
//! ADR-0014. The crate-private `convert` module bridges the two.

mod control;
mod convert;
pub mod h264;
mod ids;
mod session;
mod session_convert;
pub mod video_frame;
pub mod wire;

pub use control::{ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame};
pub use convert::{
    ConvertError, DecodeError, SCHEMA_VERSION, decode_control_frame_envelope,
    decode_envelope_length_delimited, encode_control_frame_envelope,
    encode_envelope_length_delimited,
};
pub use ids::{Generation, PrincipalId, ScopeId, SequenceNum, SessionId, VehicleId};
pub use session::{
    ClientHello, FrameRejected, FrameRejectionReason, LeaseDenialReason, LeaseRequest,
    LeaseResponse, Ping, Pong, ScopeHolderSnapshot, ServerWelcome,
};
pub use video_frame::{
    CaptureHeader, ContractFault, DecodedFrame, Offsets, encode_v2 as encode_video_frame_v2,
};
