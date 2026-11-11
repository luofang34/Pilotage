//! Typed outbound uplink lanes for the FC's GCS instance.
//!
//! The link task owns the socket's single MAVLink sequence counter and
//! does all encoding: a second sender with its own counter on the same
//! socket/identity would interleave duplicate and backward sequence
//! numbers, corrupting the packet-loss accounting the field exists for.
//! Reliable commands (claims, recenters, mode changes) ride an ordered
//! queue; gimbal rate and body velocity demands ride latest-value slots so
//! a stalled link coalesces stale demands away instead of queueing them
//! behind fresh ones.

use std::time::Instant;

use crate::codec::{AttitudeTargetKind, ParamValue};

#[cfg(feature = "net")]
mod lanes;
#[cfg(feature = "net")]
pub(super) use lanes::{
    OutboundLanes, send_gimbal_rate, send_outbound_command, send_param, send_setpoint,
};

/// One reliably-ordered COMMAND_LONG for the FC's GCS instance.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundCommand {
    /// Portable executor command checked and recorded at the final UDP write.
    pub payload_write: Option<crate::PayloadWrite>,
    /// Actual complete-write receipt for protocol policy such as primary-control claims.
    pub receipt: Option<crate::WriteReceipt>,
    /// Shared generation fence checked immediately before writing.
    pub fence: Option<super::SendFence>,
    /// Exclusive local deadline for beginning the UDP write; queues do not refresh it.
    pub send_before: Option<Instant>,
    /// MAV_CMD id.
    pub command: u16,
    /// COMMAND_LONG params 1..7.
    pub params: [f32; 7],
    /// Target MAVLink system.
    pub target_system: u8,
    /// Target MAVLink component.
    pub target_component: u8,
}

/// The latest-value gimbal rate demand (rad/s). Every publication
/// replaces the previous one; the link task encodes only the newest.
#[derive(Debug, Clone, PartialEq)]
pub struct GimbalRateDemand {
    /// Reference flags validated by the payload contract.
    pub flags: u32,
    /// Roll rate in radians per second; NaN omits the axis.
    pub roll_rps: f32,
    /// Concrete device selected from the expected manager status.
    pub gimbal_device_id: u8,
    /// Complete-write receipt for stop ordering.
    pub receipt: Option<crate::WriteReceipt>,
    /// Portable executor command checked and recorded at the final UDP write.
    pub payload_write: Option<crate::PayloadWrite>,
    /// Shared generation fence checked immediately before writing.
    pub fence: Option<super::SendFence>,
    /// Exclusive local deadline retained from the input publication.
    pub send_before: Option<Instant>,
    /// Pitch rate demand, + = camera up.
    pub pitch_rps: f32,
    /// Yaw rate demand, + = camera right.
    pub yaw_rps: f32,
    /// Target MAVLink system.
    pub target_system: u8,
    /// Target MAVLink component.
    pub target_component: u8,
}

/// The latest-value body-frame velocity demand: one
/// SET_POSITION_TARGET_LOCAL_NED in MAV_FRAME_BODY_OFFSET_NED per
/// publication. Every publication is sent, so a caller that publishes at
/// its control rate streams at that rate, and a caller that stops
/// publishing stops the stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyVelocityDemand {
    /// Velocity (forward, right, down), m/s.
    pub vel_frd_mps: [f32; 3],
    /// Yaw rate, rad/s, + = clockwise seen from above.
    pub yaw_rate_rps: f32,
    /// Target MAVLink system.
    pub target_system: u8,
    /// Target MAVLink component.
    pub target_component: u8,
}

/// The latest-value attitude-loop demand for direct flight: one
/// SET_ATTITUDE_TARGET per publication.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeThrustDemand {
    /// Body rates or an absolute attitude.
    pub kind: AttitudeTargetKind,
    /// Collective thrust in the FC's own `[0, 1]` scale.
    pub thrust: f32,
    /// Target MAVLink system.
    pub target_system: u8,
    /// Target MAVLink component.
    pub target_component: u8,
}

/// One offboard setpoint. Velocity and direct flight share one lane
/// because one FC control loop follows exactly one of them at a time: the
/// newest publication of either kind is the one the FC should follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffboardSetpoint {
    /// A body-frame velocity and yaw rate.
    BodyVelocity(BodyVelocityDemand),
    /// Body rates or attitude, with thrust.
    AttitudeThrust(AttitudeThrustDemand),
}

/// One parameter-protocol request, sent in order on its own lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamRequest {
    /// PARAM_REQUEST_READ by name.
    Read(&'static str),
    /// PARAM_SET by name.
    Set(&'static str, ParamValue),
}

/// A parameter request addressed to one FC component.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutboundParam {
    /// What to read or write.
    pub request: ParamRequest,
    /// Target MAVLink system.
    pub target_system: u8,
    /// Target MAVLink component.
    pub target_component: u8,
}
