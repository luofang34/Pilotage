//! Typed outbound uplink lanes for the FC's GCS instance.
//!
//! The link task owns the socket's single MAVLink sequence counter and
//! does all encoding: a second sender with its own counter on the same
//! socket/identity would interleave duplicate and backward sequence
//! numbers, corrupting the packet-loss accounting the field exists for.
//! Reliable commands (claims, recenters) ride an ordered queue; gimbal
//! rate demands ride a latest-value slot so a stalled link coalesces
//! stale demands away instead of queueing them behind fresh ones.

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

/// Encodes and sends one ordered command through the link socket,
/// advancing the shared sequence. A missing stream-command target drops
/// the command with a warning.
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
            if let Err(error) = socket.send_to(&frame, target).await {
                warn!(%error, "outbound command send failed");
            }
        }
        (Some(_), None) => {
            warn!("outbound command dropped: link has no stream command target");
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
        if let Err(error) = socket.send_to(&frame, target).await {
            warn!(%error, "gimbal rate setpoint send failed");
        }
    }
}
