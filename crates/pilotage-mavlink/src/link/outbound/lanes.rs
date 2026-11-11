//! The outbound lanes over a UDP socket: the receivers the link task
//! drains and the sends that encode and write each frame.

use std::net::SocketAddr;
use web_time::Instant;

use tokio::net::UdpSocket;
use tracing::warn;

use super::{GimbalRateDemand, OffboardSetpoint, OutboundCommand, OutboundParam, ParamRequest};

#[cfg(test)]
mod tests;

/// The receive ends of the outbound lanes the link task arbitrates.
#[derive(Debug)]
pub(in crate::link) struct OutboundLanes {
    pub(in crate::link) commands: tokio::sync::mpsc::Receiver<OutboundCommand>,
    pub(in crate::link) gimbal_rates: tokio::sync::watch::Receiver<Option<GimbalRateDemand>>,
    pub(in crate::link) setpoints: tokio::sync::watch::Receiver<Option<OffboardSetpoint>>,
    pub(in crate::link) params: tokio::sync::mpsc::Receiver<OutboundParam>,
}

/// Sends one encoded frame, logging a refused send with its context.
async fn send_frame(socket: &UdpSocket, frame: &[u8], target: SocketAddr, what: &str) {
    if let Err(error) = socket.send_to(frame, target).await {
        warn!(%error, %target, what, "outbound send failed");
    }
}

struct WriteContext<'a> {
    send_before: Option<Instant>,
    fence: Option<&'a crate::link::SendFence>,
    payload_write: Option<&'a crate::PayloadWrite>,
    receipt: Option<&'a crate::WriteReceipt>,
}

impl WriteContext<'_> {
    fn write(&self, operation: impl FnOnce() -> std::io::Result<bool>) -> std::io::Result<bool> {
        let write = || {
            if self
                .send_before
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Ok(false);
            }
            let sent = operation()?;
            // A revoker must see the complete-write fact when invalidate returns.
            // Record it before releasing the same fence that protects the write.
            if sent && let Some(receipt) = self.receipt {
                receipt.record_sent(Instant::now());
            }
            Ok(sent)
        };
        if let Some(fence) = self.fence {
            fence.with_valid_write(write)
        } else {
            write()
        }
    }
}

/// Readiness waits consume the original deadline before the actual socket write.
async fn send_frame_until(
    socket: &UdpSocket,
    frame: &[u8],
    target: SocketAddr,
    what: &str,
    context: WriteContext<'_>,
) {
    let WriteContext {
        send_before,
        fence,
        payload_write,
        receipt: _,
    } = context;
    loop {
        if fence.is_some_and(|fence| !fence.is_valid())
            || send_before.is_some_and(|deadline| Instant::now() >= deadline)
        {
            warn!(what, "outbound command expired before UDP write");
            return;
        }
        if let Err(error) = socket.writable().await {
            warn!(%error, what, "outbound socket readiness failed");
            return;
        }
        if fence.is_some_and(|fence| !fence.is_valid())
            || send_before.is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }
        let result = context.write(|| {
            if let Some(write) = payload_write {
                write.write(frame.len(), || socket.try_send_to(frame, target))
            } else {
                socket
                    .try_send_to(frame, target)
                    .map(|written| written == frame.len())
            }
        });
        match result {
            Ok(_) => return,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) => {
                warn!(%error, what, "outbound send failed");
                return;
            }
        }
    }
}

/// Encodes and sends one ordered command through the link socket,
/// advancing the shared sequence. A missing target drops the command
/// with a warning.
pub(in crate::link) async fn send_outbound_command(
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
            send_frame_until(
                socket,
                &frame,
                target,
                "command",
                WriteContext {
                    send_before: command.send_before,
                    fence: command.fence.as_ref(),
                    payload_write: command.payload_write.as_ref(),
                    receipt: command.receipt.as_ref(),
                },
            )
            .await;
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
pub(in crate::link) async fn send_gimbal_rate(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    demand: Option<GimbalRateDemand>,
) {
    if let (Some(demand), Some(target)) = (demand, target) {
        let frame = crate::codec::encode_gimbal_rate_setpoint(
            *seq,
            demand.flags,
            [demand.roll_rps, demand.pitch_rps, demand.yaw_rps],
            demand.target_system,
            demand.target_component,
            demand.gimbal_device_id,
        );
        *seq = seq.wrapping_add(1);
        send_frame_until(
            socket,
            &frame,
            target,
            "gimbal rate setpoint",
            WriteContext {
                send_before: demand.send_before,
                fence: demand.fence.as_ref(),
                payload_write: demand.payload_write.as_ref(),
                receipt: demand.receipt.as_ref(),
            },
        )
        .await;
    }
}

/// Encodes and sends one offboard setpoint, advancing the shared
/// sequence. `None` demand sends nothing.
pub(in crate::link) async fn send_setpoint(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    time_boot_ms: u32,
    setpoint: Option<OffboardSetpoint>,
) {
    let (Some(setpoint), Some(target)) = (setpoint, target) else {
        return;
    };
    match setpoint {
        OffboardSetpoint::BodyVelocity(demand) => {
            let frame = crate::codec::encode_body_velocity_setpoint(
                *seq,
                time_boot_ms,
                demand.vel_frd_mps,
                demand.yaw_rate_rps,
                demand.target_system,
                demand.target_component,
            );
            send_frame(socket, &frame, target, "body velocity setpoint").await;
        }
        OffboardSetpoint::AttitudeThrust(demand) => {
            let frame = crate::codec::encode_attitude_target(
                *seq,
                time_boot_ms,
                demand.kind,
                demand.thrust,
                demand.target_system,
                demand.target_component,
            );
            send_frame(socket, &frame, target, "attitude target").await;
        }
    }
    *seq = seq.wrapping_add(1);
}

/// Encodes and sends one parameter request, advancing the shared
/// sequence. A missing target drops it with a warning.
pub(in crate::link) async fn send_param(
    socket: &UdpSocket,
    seq: &mut u8,
    target: Option<SocketAddr>,
    param: Option<OutboundParam>,
) {
    let Some(param) = param else {
        return;
    };
    let Some(target) = target else {
        warn!(
            ?param,
            "parameter request dropped: link has no command target yet"
        );
        return;
    };
    match param.request {
        ParamRequest::Read(name) => {
            let frame = crate::codec::encode_param_request_read(
                *seq,
                name,
                param.target_system,
                param.target_component,
            );
            send_frame(socket, &frame, target, "parameter read").await;
        }
        ParamRequest::Set(name, value) => {
            let frame = crate::codec::encode_param_set(
                *seq,
                name,
                value,
                param.target_system,
                param.target_component,
            );
            send_frame(socket, &frame, target, "parameter set").await;
        }
    }
    *seq = seq.wrapping_add(1);
}
