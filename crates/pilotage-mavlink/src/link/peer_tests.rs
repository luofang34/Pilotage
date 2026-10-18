#![allow(clippy::expect_used, clippy::panic)]

//! Socket-level proof of [`CommandTarget::FcSender`]: the link sends
//! nothing until the selected FC speaks, then sends its heartbeat,
//! interval requests, commands, and setpoints to the address the FC spoke
//! from. A datagram from another system teaches it nothing.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::codec::{
    COMMAND_LONG_ID, HEARTBEAT_ID, MAV_CMD_NAV_TAKEOFF, SET_POSITION_TARGET_ID, encode_command_ack,
};

use super::outbound::OutboundLanes;
use super::{BodyVelocityDemand, CommandTarget, LinkConfig, LinkState, OutboundCommand, run_link};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

fn message_id(frame: &[u8]) -> u32 {
    assert!(frame.len() >= 10 && frame[0] == 0xFD, "a MAVLink v2 frame");
    u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16)
}

/// Collects message ids arriving at `socket` until `wanted` all appeared,
/// failing the test once a deadline passes.
async fn receive_until(socket: &UdpSocket, wanted: &[u32]) -> Vec<u32> {
    let mut seen = Vec::new();
    let mut buf = [0u8; 128];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !wanted.iter().all(|id| seen.contains(id)) {
        let (len, _) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("only {seen:?} arrived, wanted {wanted:?}"))
            .expect("a datagram");
        seen.push(message_id(&buf[..len]));
    }
    seen
}

struct Harness {
    link_addr: SocketAddr,
    commands: tokio::sync::mpsc::Sender<OutboundCommand>,
    velocity: tokio::sync::watch::Sender<Option<BodyVelocityDemand>>,
    _rates: tokio::sync::watch::Sender<Option<super::GimbalRateDemand>>,
    task: tokio::task::JoinHandle<()>,
}

async fn start_link() -> Harness {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("link binds");
    let link_addr = socket.local_addr().expect("link address");
    let config = LinkConfig {
        command_target: CommandTarget::FcSender,
        stream_interval_requests: &[(31, 33_333)],
        ..LinkConfig::default()
    };
    let (commands, command_rx) = tokio::sync::mpsc::channel(8);
    let (rates, rate_rx) = tokio::sync::watch::channel(None);
    let (velocity, velocity_rx) = tokio::sync::watch::channel(None);
    let lanes = OutboundLanes {
        commands: command_rx,
        gimbal_rates: rate_rx,
        body_velocity: velocity_rx,
    };
    let state = Arc::new(Mutex::new(LinkState::default()));
    let task = tokio::spawn(run_link(socket, config, false, state, lanes));
    Harness {
        link_addr,
        commands,
        velocity,
        _rates: rates,
        task,
    }
}

#[test]
fn the_link_answers_the_address_the_fc_spoke_from() {
    runtime().block_on(async {
        let harness = start_link().await;
        // A connected socket, as ArduPilot SITL's udpclient port is.
        let fc = UdpSocket::bind("127.0.0.1:0").await.expect("fc binds");
        fc.connect(harness.link_addr).await.expect("fc connects");
        fc.send(&encode_command_ack(0, 0, 0, 1, 1))
            .await
            .expect("the fc speaks");
        // The heartbeat proves the link learned this address; anything
        // queued before that has nowhere to go.
        receive_until(&fc, &[HEARTBEAT_ID]).await;

        harness
            .commands
            .send(OutboundCommand {
                command: MAV_CMD_NAV_TAKEOFF,
                params: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.5],
                target_system: 1,
                target_component: 1,
            })
            .await
            .expect("the command queues");
        harness
            .velocity
            .send(Some(BodyVelocityDemand {
                vel_frd_mps: [1.0, 0.0, 0.0],
                yaw_rate_rps: 0.0,
                target_system: 1,
                target_component: 1,
            }))
            .expect("the setpoint publishes");

        receive_until(&fc, &[COMMAND_LONG_ID, SET_POSITION_TARGET_ID]).await;
        harness.task.abort();
    });
}

#[test]
fn a_stranger_does_not_become_the_command_target() {
    runtime().block_on(async {
        let harness = start_link().await;
        let stranger = UdpSocket::bind("127.0.0.1:0").await.expect("binds");
        // System 2 is not the selected FC.
        stranger
            .send_to(&encode_command_ack(0, 0, 0, 2, 1), harness.link_addr)
            .await
            .expect("the stranger speaks");
        harness
            .commands
            .send(OutboundCommand {
                command: MAV_CMD_NAV_TAKEOFF,
                params: [0.0; 7],
                target_system: 1,
                target_component: 1,
            })
            .await
            .expect("the command queues");

        let mut buf = [0u8; 128];
        // Two heartbeat periods: long enough for a heartbeat, an interval
        // request, and the command, had the stranger been learned.
        let heard =
            tokio::time::timeout(Duration::from_millis(2_200), stranger.recv_from(&mut buf)).await;
        harness.task.abort();
        assert!(heard.is_err(), "the stranger received a frame");
    });
}
