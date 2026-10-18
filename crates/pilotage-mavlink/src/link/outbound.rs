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

use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tracing::warn;

/// One reliably-ordered COMMAND_LONG for the FC's GCS instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutboundCommand {
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalRateDemand {
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

/// The receive ends of the outbound lanes the link task arbitrates.
#[derive(Debug)]
pub(super) struct OutboundLanes {
    pub(super) commands: tokio::sync::mpsc::Receiver<OutboundCommand>,
    pub(super) gimbal_rates: tokio::sync::watch::Receiver<Option<GimbalRateDemand>>,
    pub(super) body_velocity: tokio::sync::watch::Receiver<Option<BodyVelocityDemand>>,
}

/// Sends one encoded frame, logging a refused send with its context.
async fn send_frame(socket: &UdpSocket, frame: &[u8], target: SocketAddr, what: &str) {
    if let Err(error) = socket.send_to(frame, target).await {
        warn!(%error, %target, what, "outbound send failed");
    }
}

/// Encodes and sends one ordered command through the link socket,
/// advancing the shared sequence. A missing target drops the command
/// with a warning.
pub(super) async fn send_outbound_command(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    command: Option<OutboundCommand>,
) {
    match (command, target) {
        (Some(command), Some(target)) => {
            let frame = crate::codec::encode_command_long(
                *seq,
                command.command,
                command.params,
                command.target_system,
                command.target_component,
            );
            *seq = seq.wrapping_add(1);
            send_frame(socket, &frame, target, "command").await;
        }
        (Some(command), None) => {
            warn!(
                command = command.command,
                "outbound command dropped: link has no command target yet"
            );
        }
        // All senders dropped; the link is being torn down.
        (None, _) => {}
    }
}

/// Encodes and sends one gimbal rate setpoint, advancing the shared
/// sequence. `None` demand (a cleared lane) sends nothing.
pub(super) async fn send_gimbal_rate(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    demand: Option<GimbalRateDemand>,
) {
    if let (Some(demand), Some(target)) = (demand, target) {
        let frame = crate::codec::encode_gimbal_rate_setpoint(
            *seq,
            demand.pitch_rps,
            demand.yaw_rps,
            demand.target_system,
            demand.target_component,
        );
        *seq = seq.wrapping_add(1);
        send_frame(socket, &frame, target, "gimbal rate setpoint").await;
    }
}

/// Encodes and sends one body velocity setpoint, advancing the shared
/// sequence. `None` demand sends nothing.
pub(super) async fn send_body_velocity(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    time_boot_ms: u32,
    demand: Option<BodyVelocityDemand>,
) {
    if let (Some(demand), Some(target)) = (demand, target) {
        let frame = crate::codec::encode_body_velocity_setpoint(
            *seq,
            time_boot_ms,
            demand.vel_frd_mps,
            demand.yaw_rate_rps,
            demand.target_system,
            demand.target_component,
        );
        *seq = seq.wrapping_add(1);
        send_frame(socket, &frame, target, "body velocity setpoint").await;
    }
}
