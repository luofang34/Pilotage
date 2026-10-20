use super::*;

#[test]
fn queued_expired_and_revoked_commands_do_not_reach_the_socket() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = receiver.local_addr().unwrap();
        let fence = crate::SendFence::default();
        let command = OutboundCommand {
            payload_write: None,
            receipt: None,
            fence: Some(fence.clone()),
            send_before: Some(Instant::now() + std::time::Duration::from_secs(1)),
            command: 531,
            params: [0.0; 7],
            target_system: 1,
            target_component: 100,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.send(command).await.unwrap();
        fence.invalidate();
        let mut sequence = 0;
        send_outbound_command(&sender, &mut sequence, Some(target), rx.recv().await).await;
        let mut bytes = [0; 300];
        assert_eq!(
            receiver.try_recv(&mut bytes).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let expired = OutboundCommand {
            payload_write: None,
            receipt: None,
            fence: None,
            send_before: Some(Instant::now()),
            command: 531,
            params: [0.0; 7],
            target_system: 1,
            target_component: 100,
        };
        send_outbound_command(&sender, &mut sequence, Some(target), Some(expired)).await;
        assert_eq!(
            receiver.try_recv(&mut bytes).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let fresh = OutboundCommand {
            payload_write: None,
            receipt: None,
            fence: Some(crate::SendFence::default()),
            send_before: Some(Instant::now() + std::time::Duration::from_secs(1)),
            command: 531,
            params: [0.0; 7],
            target_system: 1,
            target_component: 100,
        };
        send_outbound_command(&sender, &mut sequence, Some(target), Some(fresh)).await;
        let received =
            tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv(&mut bytes))
                .await
                .unwrap()
                .unwrap();
        assert!(received > 0);
    });
}
