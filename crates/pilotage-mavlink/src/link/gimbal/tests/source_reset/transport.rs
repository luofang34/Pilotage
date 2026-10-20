use super::*;
use crate::link::outbound::{send_gimbal_rate, send_outbound_command};
use crate::{ManagerControl, OutboundCommand};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

mod lifecycle;

async fn receive_frame(receiver: &UdpSocket) -> (u32, Vec<u8>) {
    let mut bytes = [0; 300];
    let length = tokio::time::timeout(Duration::from_secs(1), receiver.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(length >= 12);
    (
        u32::from_le_bytes([bytes[7], bytes[8], bytes[9], 0]),
        bytes[10..length - 2].to_vec(),
    )
}

async fn receive_message(receiver: &UdpSocket) -> u32 {
    receive_frame(receiver).await.0
}

async fn sentinel(sender: &UdpSocket, receiver: &UdpSocket, sequence: &mut u8) {
    send_outbound_command(
        sender,
        sequence,
        Some(receiver.local_addr().unwrap()),
        Some(OutboundCommand {
            command: 512,
            params: [280.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            target_system: 1,
            target_component: KEY.0,
            send_before: Some(Instant::now() + Duration::from_secs(1)),
            fence: None,
            payload_write: None,
            receipt: None,
        }),
    )
    .await;
    let (message, payload) = receive_frame(receiver).await;
    assert_eq!(message, 76, "no stale rate precedes the ordered sentinel");
    assert_eq!(u16::from_le_bytes([payload[28], payload[29]]), 512);
    assert_eq!(
        f32::from_le_bytes(payload[..4].try_into().unwrap()),
        280.0,
        "no stale claim or release precedes the ordered sentinel"
    );
}

async fn queued_rate_after_source_reset(restart: bool) {
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let state = Arc::new(Mutex::new(LinkState {
        reset_policy: ResetPolicy::SimulatorHeuristic,
        ..LinkState::default()
    }));
    let (commands, _pending_commands) = mpsc::channel(16);
    let (rates, observer) = watch::channel(None);
    let mut control = ManagerControl::new(commands, rates, 1, Some(KEY))
        .unwrap()
        .with_manager_state(state.clone());
    prepare_source_reset(&state, restart);
    discovered(&state, 100, OWNER);
    assert!(control.rate_demand(0.5, 0.0));
    let queued = observer.borrow().clone().unwrap();
    assert!(queued.send_before.unwrap() > Instant::now());
    finish_source_reset(&state, restart);
    let target = Some(receiver.local_addr().unwrap());
    let mut sequence = 0;
    send_gimbal_rate(&sender, &mut sequence, target, Some(queued.clone())).await;
    sentinel(&sender, &receiver, &mut sequence).await;
    assert!(
        control
            .snapshot_cached()
            .unwrap()
            .progress
            .sent_at
            .is_none()
    );
    discovered(&state, 103, OWNER);
    assert!(
        queued.send_before.unwrap() > Instant::now(),
        "identity, not timeout, must reject"
    );
    send_gimbal_rate(&sender, &mut sequence, target, Some(queued)).await;
    sentinel(&sender, &receiver, &mut sequence).await;
    assert!(
        control
            .snapshot_cached()
            .unwrap()
            .progress
            .sent_at
            .is_none()
    );
    control.maintain();
    assert!(control.rate_demand(0.5, 0.0));
    let fresh = observer.borrow().clone().unwrap();
    send_gimbal_rate(&sender, &mut sequence, target, Some(fresh)).await;
    assert_eq!(receive_message(&receiver).await, 282);
    assert!(
        control
            .snapshot_cached()
            .unwrap()
            .progress
            .sent_at
            .is_some()
    );
}

#[test]
fn queued_rate_cannot_cross_a_raw_fc_wrap_or_confirmed_reset_on_the_udp_writer() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for restart in [false, true] {
            queued_rate_after_source_reset(restart).await;
        }
    });
}
