//! Wire-level identifiers and control-frame vocabulary for Pilotage.
//!
//! Types generated from `schemas/` live in [`wire`] alongside the
//! hand-written identifier and control types below, per ADR-0002 and
//! ADR-0014. The crate-private `convert` module bridges the two.

mod command_outcome;
mod control;
mod convert;
mod flight_procedure;
mod gimbal_reference;
pub mod h264;
mod heading;
mod ids;
mod intent;
mod join_token;
mod landed_state;
mod payload;
mod scopes;
mod session;
mod session_convert;
mod timing;
mod vehicle_class;
pub mod video_frame;
pub mod wire;
mod zoom;

pub use command_outcome::{COMMAND_ANSWER_WINDOW_MS, CommandOutcome};
pub use control::{ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame};
pub use convert::{
    ConvertError, DecodeError, SCHEMA_VERSION, decode_action_command_envelope,
    decode_control_frame_envelope, decode_envelope_length_delimited, encode_control_frame_envelope,
    encode_envelope_length_delimited,
};
pub use flight_procedure::{FlightProcedureKind, FlightProcedurePhase, FlightProcedureReason};
pub use gimbal_reference::{GimbalAxisLocks, GimbalReference, GimbalYawFrame};
pub use heading::{HeadingReference, MagneticVariationSource};
pub use ids::{Generation, PrincipalId, ScopeId, SequenceNum, SessionId, VehicleId};
pub use intent::{
    ActionKind, AttitudeThrustIntent, BodyRateIntent, ControlAction, ControlIntent, FeelTarget,
    GimbalRateIntent, IntentFamily, ModeTarget, PositionHoldIntent, ReferenceFrame, VelocityIntent,
    actions_conflict,
};
pub use join_token::{ADMISSION_REJECTED_CLOSE_CODE, JoinToken, JoinTokenError};
pub use landed_state::LandedState;
pub use scopes::{DIRECT_SCOPE, MOTION_SCOPE};
pub use session::{
    ClientHello, ControlActionCommand, ControlActionResult, FrameRejected, FrameRejectionReason,
    LeaseDenialReason, LeaseRelease, LeaseReleased, LeaseRequest, LeaseResponse, LinkLossCleared,
    Ping, Pong, ProfileActivation, SESSION_PROTOCOL_VERSION, ScopeHolderSnapshot,
    ScopeTransferAccept, ScopeTransferOffer, ScopeTransferRequest, ServerWelcome,
};
pub use vehicle_class::VehicleClass;
pub use video_frame::{
    CaptureHeader, ContractFault, DecodedFrame, Offsets, encode_v2 as encode_video_frame_v2,
};

pub use zoom::{ZoomCommand, ZoomUnit};

pub use payload::{payload_rate_limit, payload_zoom_admits};

pub use timing::{AdmittedTiming, CommandTiming};
