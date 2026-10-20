//! MAVLink 2.0 subset codec and receive-link machinery shared by
//! flight-controller adapters (ADR-0018).
//!
//! The [`codec`] module is pure byte functions — framing, CRC-EXTRA,
//! decoding of the standard message subset (heartbeat, attitude,
//! local position, command ack, estimator status) plus the Aviate
//! private estimator-status extension — unit-testable byte-for-byte.
//! The [`link`] module owns the UDP receive task and the shared
//! latest-state cache with the acquisition-clock discipline: source
//! epochs, reboot detection, inter-group skew budgets, and staleness
//! stamps. Adapters sample the cache; they never touch the socket.

pub mod ardupilot;
pub mod codec;
mod command_result;
pub mod gimbal_control;
pub mod gimbal_endpoint;
mod gimbal_flags;
pub use gimbal_control::ManagerControl;
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
    CameraReport, CommandAckReport, CommandTarget, FcReports, GimbalDeviceAttitude,
    GimbalManagerReport, GimbalRateDemand, GimbalReports, KinematicsUpdate, LandedStateReport,
    LinkConfig, LinkError, LinkState, ManagerWriteRequirement, MavlinkLink, OffboardSetpoint,
    OutboundCommand, OutboundParam, ParamRequest, RelativeAltitudeReport, ResetPolicy, SendFence,
    StatusTextReport, TakeoffWriteRequirement,
};
