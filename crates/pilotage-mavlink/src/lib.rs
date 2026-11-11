//! MAVLink 2.0 subset codec and receive-link machinery shared by
//! flight-controller adapters (ADR-0018).
//!
//! The [`codec`] module is pure byte functions — framing, CRC-EXTRA,
//! decoding of the standard message subset (heartbeat, attitude,
//! local position, command ack, estimator status) plus the Aviate
//! private estimator-status extension — unit-testable byte-for-byte.
//! The [`link`] module owns the shared latest-state cache with the
//! acquisition-clock discipline: source epochs, reboot detection,
//! inter-group skew budgets, and staleness stamps. Adapters sample the
//! cache; they never touch the socket.
//!
//! The `net` feature (on by default) adds the tokio side: the UDP
//! receive task and outbound lanes ([`MavlinkLink`]), the camera
//! endpoints, and the gimbal endpoint and manager control. Without it the crate builds
//! for `wasm32-unknown-unknown`, and a host feeds the cache itself with
//! [`link::apply_messages_at`].

pub mod ardupilot;
#[cfg(feature = "net")]
pub mod camera_endpoint;
pub mod codec;
mod command_result;
#[cfg(feature = "net")]
pub mod gimbal_control;
#[cfg(feature = "net")]
pub mod gimbal_endpoint;
mod gimbal_flags;
#[cfg(feature = "net")]
pub use camera_endpoint::CameraEndpoints;
#[cfg(feature = "net")]
pub use gimbal_control::ManagerControl;
#[cfg(feature = "net")]
pub use link::MavlinkLink;
pub mod link;
mod payload_execution;
pub mod sampling;
pub use payload_execution::{
    PayloadChannel, PayloadEffect, PayloadExecutionError, PayloadSnapshot, PayloadTiming,
    PayloadWrite, WriteReceipt,
};

pub use codec::{FcMessage, FrameSource, LandedState, ParseStats, parse_datagram};
pub use command_result::command_outcome;
pub use gimbal_flags::gimbal_reference;
pub use link::{
    AttitudeThrustDemand, AttitudeUpdate, AuthorizationSource, BatteryReport, BodyVelocityDemand,
    CameraReport, CameraWriteRequirement, CommandAckReport, CommandTarget, FcReports,
    GimbalDeviceAttitude, GimbalManagerReport, GimbalRateDemand, GimbalReports, KinematicsUpdate,
    LandedStateReport, LinkConfig, LinkError, LinkState, ManagerWriteRequirement, OffboardSetpoint,
    OutboundCommand, OutboundParam, ParamRequest, RelativeAltitudeReport, ResetPolicy, SendFence,
    StatusTextReport, TakeoffWriteRequirement,
};
