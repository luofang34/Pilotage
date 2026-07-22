#![allow(clippy::expect_used, clippy::panic)]

//! COMMAND_ACK enactment truth: PX4 acknowledges a COMMAND_LONG to the
//! endpoint that sent it, so the arm/disarm verdict pairs on the command
//! socket and must survive to [`Px4Uplink::last_arm_ack`].

use std::net::UdpSocket;
use std::time::Duration;

use pilotage_adapter_api::FcCommandAck;
use pilotage_mavlink::codec::encode_command_ack;

use super::{fake_fc, uplink_to};
use crate::uplink::Px4Uplink;

/// Drains FC replies (one `maintain` per pass) until `condition` holds or
/// a deadline lapses — loopback UDP delivery has no event to await.
fn drain_until(uplink: &mut Px4Uplink, condition: impl Fn(&Px4Uplink) -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        uplink.maintain();
        if condition(uplink) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// Receives frames at the fake FC until the arm/disarm COMMAND_LONG
/// arrives, returning the uplink socket's address to acknowledge to.
fn recv_until_arm_command(fc: &UdpSocket) -> std::net::SocketAddr {
    let mut buf = [0u8; 128];
    loop {
        let (len, from) = fc.recv_from(&mut buf).expect("datagram");
        assert!(len > 10, "runt frame");
        let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
        if msg_id == 76 && u16::from_le_bytes([buf[10 + 28], buf[10 + 29]]) == 400 {
            return from;
        }
    }
}

#[test]
fn an_fc_command_ack_pairs_with_the_latest_arm_or_disarm() {
    let (fc, addr) = fake_fc();
    let mut uplink = uplink_to(addr);
    uplink.use_manual_clock();
    uplink.begin_arm(0.0);
    uplink.advance_clock(Duration::from_millis(400));
    uplink.maintain();
    let uplink_addr = recv_until_arm_command(&fc);
    assert_eq!(
        uplink.last_arm_ack(),
        None,
        "no verdict before the FC answers"
    );

    // The FC refuses the arm (MAV_RESULT 4, TEMPORARILY_REJECTED): the
    // refusal must surface, not vanish into a fire-and-forget socket.
    let refused = encode_command_ack(0, 400, 4, 1, 1);
    fc.send_to(&refused, uplink_addr).expect("ack send");
    assert!(
        drain_until(&mut uplink, |u| u.last_arm_ack().is_some()),
        "the refusal reaches the uplink"
    );
    assert_eq!(
        uplink.last_arm_ack(),
        Some(FcCommandAck {
            arm: true,
            result: 4
        })
    );

    // A new disarm opens a fresh verdict slot; the FC accepts this one.
    uplink.send_disarm();
    assert_eq!(
        uplink.last_arm_ack(),
        None,
        "a fresh command clears the stale verdict"
    );
    let accepted = encode_command_ack(1, 400, 0, 1, 1);
    fc.send_to(&accepted, uplink_addr).expect("ack send");
    assert!(
        drain_until(&mut uplink, |u| u.last_arm_ack().is_some()),
        "the acceptance reaches the uplink"
    );
    assert_eq!(
        uplink.last_arm_ack(),
        Some(FcCommandAck {
            arm: false,
            result: 0
        })
    );
}
