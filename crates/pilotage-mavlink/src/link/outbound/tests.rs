use super::*;

#[test]
fn revocation_observes_the_complete_write_receipt() {
    let fence = crate::SendFence::default();
    let receipt = crate::WriteReceipt::default();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer_fence = fence.clone();
    let writer_receipt = receipt.clone();
    let writer = std::thread::spawn(move || {
        WriteContext {
            send_before: None,
            fence: Some(&writer_fence),
            payload_write: None,
            receipt: Some(&writer_receipt),
        }
        .write(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(true)
        })
        .unwrap()
    });
    entered_rx.recv().unwrap();
    let (revoke_tx, revoke_rx) = std::sync::mpsc::channel();
    let revoker = std::thread::spawn(move || {
        revoke_tx.send(()).unwrap();
        fence.invalidate();
        receipt.sent_at_cached()
    });
    revoke_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    assert!(writer.join().unwrap());
    assert!(revoker.join().unwrap().is_some());
}

#[test]
fn incomplete_or_refused_writes_do_not_create_complete_write_receipts() {
    let receipt = crate::WriteReceipt::default();
    let fence = crate::SendFence::default();
    let context = WriteContext {
        send_before: None,
        fence: Some(&fence),
        payload_write: None,
        receipt: Some(&receipt),
    };
    assert!(!context.write(|| Ok(false)).unwrap());
    assert!(receipt.sent_at_cached().is_none());
    assert!(
        context
            .write(|| Err(std::io::ErrorKind::WouldBlock.into()))
            .is_err()
    );
    assert!(receipt.sent_at_cached().is_none());
    fence.invalidate();
    assert!(
        !context
            .write(|| panic!("revoked write must not run"))
            .unwrap()
    );
    assert!(receipt.sent_at_cached().is_none());
}

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
