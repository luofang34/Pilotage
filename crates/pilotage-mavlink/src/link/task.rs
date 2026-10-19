//! The link task: one socket, one MAVLink sequence, and the arbitration
//! between FC telemetry and the outbound lanes.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tracing::{error, warn};

use super::apply::apply_messages;
use super::outbound::{
    OutboundLanes, send_gimbal_rate, send_outbound_command, send_param, send_setpoint,
};
use super::{CommandTarget, LinkConfig, LinkState};
use crate::codec::{
    FcMessage, FrameSource, MAV_CMD_SET_MESSAGE_INTERVAL, encode_gcs_heartbeat, parse_datagram,
};

/// The address outbound frames go to now, resolved from the configured
/// target and the FC's last sender address.
fn resolve_target(target: CommandTarget, fc_sender: Option<SocketAddr>) -> Option<SocketAddr> {
    match target {
        CommandTarget::None => None,
        CommandTarget::Fixed(address) => Some(address),
        CommandTarget::FcSender => fc_sender,
    }
}

pub(super) async fn run_link(
    socket: UdpSocket,
    config: LinkConfig,
    router_mode: bool,
    state: Arc<Mutex<LinkState>>,
    mut lanes: OutboundLanes,
) {
    let started = Instant::now();
    let mut buf = vec![0u8; 2048];
    let mut messages = Vec::with_capacity(8);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut seq: u8 = 0;
    let mut rates_open = true;
    let mut setpoints_open = true;
    let mut fc_sender: Option<SocketAddr> = None;
    loop {
        let target = resolve_target(config.command_target, fc_sender);
        tokio::select! {
            // Biased: the command lane is polled before the setpoint lanes
            // so a queued command always reaches the wire before a
            // setpoint enqueued after it. Without this, an unbiased select
            // could transmit the first gimbal rate demand ahead of its claim
            // and PX4 would drop it as non-primary
            // (claim-before-first-setpoint).
            biased;
            command = lanes.commands.recv() => {
                send_outbound_command(&socket, &mut seq, target, command).await;
            }
            changed = lanes.gimbal_rates.changed(), if rates_open => {
                if changed.is_err() {
                    // The demand producer dropped; stop polling the lane.
                    rates_open = false;
                } else {
                    let demand = lanes.gimbal_rates.borrow_and_update().clone();
                    send_gimbal_rate(&socket, &mut seq, target, demand).await;
                }
            }
            param = lanes.params.recv() => {
                send_param(&socket, &mut seq, target, param).await;
            }
            changed = lanes.setpoints.changed(), if setpoints_open => {
                if changed.is_err() {
                    setpoints_open = false;
                } else {
                    let setpoint = *lanes.setpoints.borrow_and_update();
                    let time_boot_ms =
                        u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
                    send_setpoint(&socket, &mut seq, target, time_boot_ms, setpoint).await;
                }
            }
            _ = heartbeat.tick() => {
                heartbeat_tick(&socket, &mut seq, &config, router_mode, target).await;
            }
            received = socket.recv_from(&mut buf) => {
                match received {
                    Ok((len, from)) => {
                        let from_fc = fold_datagram(
                            buf.get(..len).unwrap_or(&[]),
                            &config,
                            &state,
                            &mut messages,
                        );
                        if from_fc {
                            fc_sender = Some(from);
                        }
                    }
                    Err(error) => {
                        warn!(%error, "MAVLink socket receive failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}

/// The 1 Hz presence and stream upkeep: the GCS heartbeat (to a router,
/// or to an FC whose address the link learned) and the message-interval
/// requests.
async fn heartbeat_tick(
    socket: &UdpSocket,
    seq: &mut u8,
    config: &LinkConfig,
    router_mode: bool,
    target: Option<SocketAddr>,
) {
    let heartbeat_target = if router_mode {
        Some(config.endpoint)
    } else if config.command_target == CommandTarget::FcSender {
        target
    } else {
        None
    };
    if let Some(heartbeat_target) = heartbeat_target {
        let frame = encode_gcs_heartbeat(*seq);
        *seq = seq.wrapping_add(1);
        if let Err(error) = socket.send_to(&frame, heartbeat_target).await {
            warn!(%error, "GCS heartbeat send failed");
        }
    }
    let Some(target) = target else {
        return;
    };
    for &(message_id, interval_us) in config.stream_interval_requests {
        let frame = crate::codec::encode_command_long(
            *seq,
            MAV_CMD_SET_MESSAGE_INTERVAL,
            [
                message_id as f32,
                interval_us as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            config.system_id,
            config.component_id,
        );
        *seq = seq.wrapping_add(1);
        if let Err(error) = socket.send_to(&frame, target).await {
            warn!(%error, "stream interval request send failed");
        }
    }
}

/// Parses one datagram into the reusable message buffer, folds it into
/// the shared cache, and surfaces integrity failures loudly. Returns
/// whether the datagram carried a frame from the selected FC.
fn fold_datagram(
    bytes: &[u8],
    config: &LinkConfig,
    state: &Arc<Mutex<LinkState>>,
    messages: &mut Vec<(FrameSource, FcMessage)>,
) -> bool {
    messages.clear();
    let stats = parse_datagram(bytes, messages);
    apply_messages(state, messages, stats.crc_failures, stats.unknown_ids);
    if stats.crc_failures > 0 {
        warn!(
            crc_failures = stats.crc_failures,
            "MAVLink CRC failures in datagram"
        );
    }
    if stats.invalid_estimator_status_frames > 0 {
        error!(
            invalid_frames = stats.invalid_estimator_status_frames,
            "private estimator status frame failed validation"
        );
    }
    messages.iter().any(|(source, _)| {
        source.system_id == config.system_id && source.component_id == config.component_id
    })
}
