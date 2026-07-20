#![allow(clippy::expect_used, clippy::panic)]

//! Socket-level proof of the outbound arbiter's ordering guarantee: when
//! a gimbal claim (CONFIGURE) and a rate setpoint are both pending, the
//! `biased` select in [`run_link`] must transmit the claim FIRST. PX4
//! drops a rate demand that arrives before the primary-control claim, so
//! this is verified end-to-end over a real UDP socket — not by
//! inspecting the command and rate lanes in isolation, which cannot see
//! how the arbiter interleaves them onto the wire.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::codec::{
    COMMAND_LONG_ID, GIMBAL_MANAGER_SET_ATTITUDE_ID, MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE,
};

use super::{GimbalRateDemand, LinkConfig, LinkState, OutboundCommand, run_link};

/// Receives one datagram and returns its MAVLink v2 message id, failing
/// the test if nothing arrives promptly or the frame is not v2.
async fn recv_frame(socket: &UdpSocket) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    let (len, _from) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buf))
        .await
        .expect("a datagram arrives within 2s")
        .expect("the datagram is received");
    (buf, len)
}

fn message_id(frame: &[u8]) -> u32 {
    assert!(frame.len() >= 10 && frame[0] == 0xFD, "a MAVLink v2 frame");
    u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16)
}

/// The COMMAND_LONG `command` field: params 1..7 (28 bytes) then the u16
/// command, at payload offset 28 — frame offset 38 after the 10-byte v2
/// header.
fn command_long_command(frame: &[u8]) -> u16 {
    assert!(frame.len() >= 40, "a full COMMAND_LONG frame");
    u16::from(frame[38]) | (u16::from(frame[39]) << 8)
}

#[tokio::test]
async fn configure_reaches_the_wire_before_the_first_rate_frame() {
    // The FC's socket receives whatever the link transmits.
    let fc = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("the fc socket binds");
    let fc_addr = fc.local_addr().expect("the fc address is readable");
    let link_socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("the link socket binds");

    // router_mode = false and no stream-interval requests, so the heartbeat
    // tick emits nothing: the only datagrams are the two under test.
    let config = LinkConfig {
        stream_command_target: Some(fc_addr),
        ..LinkConfig::default()
    };
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (command_tx, command_rx) = tokio::sync::mpsc::channel(8);
    let (rate_tx, rate_rx) = tokio::sync::watch::channel(None);

    // Enqueue the claim, THEN the rate — both are pending before the
    // arbiter first runs, so only the biased ordering decides which wins.
    command_tx
        .send(OutboundCommand {
            command: MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE,
            params: [0.0; 7],
            target_system: 1,
            target_component: 1,
        })
        .await
        .expect("the claim queues");
    rate_tx
        .send(Some(GimbalRateDemand {
            pitch_rps: 0.2,
            yaw_rps: -0.1,
            target_system: 1,
            target_component: 1,
        }))
        .expect("the rate publishes");

    let link = tokio::spawn(run_link(
        link_socket,
        config,
        false,
        state,
        command_rx,
        rate_rx,
    ));

    let (first, first_len) = recv_frame(&fc).await;
    let (second, second_len) = recv_frame(&fc).await;
    link.abort();

    assert_eq!(
        message_id(&first[..first_len]),
        COMMAND_LONG_ID,
        "the claim (a COMMAND_LONG) reaches the wire first"
    );
    assert_eq!(
        command_long_command(&first[..first_len]),
        MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE,
        "the first frame is specifically the gimbal-manager CONFIGURE claim"
    );
    assert_eq!(
        message_id(&second[..second_len]),
        GIMBAL_MANAGER_SET_ATTITUDE_ID,
        "the rate setpoint follows the claim, never ahead of it"
    );
}
