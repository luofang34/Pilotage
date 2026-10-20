use super::*;

fn wait_until(deadline: Instant) {
    let (retained, elapsed) = mpsc::sync_channel::<()>(0);
    assert!(matches!(
        elapsed.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(retained);
}

#[test]
fn owner_does_not_extend_a_business_deadline_while_polling() {
    let (release, gate) = mpsc::sync_channel(1);
    let (mut usb, harness) = setup_with_gate(Some(gate));
    harness.polled.recv().expect("owner held before dequeue");
    let (queued, original) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(20);
        queued.send(deadline).expect("business deadline");
        let error = usb.try_write_until(&[1], deadline).expect_err("expired");
        usb.close_blocking().expect("closed");
        error
    });
    wait_until(original.recv().expect("caller starts"));
    release
        .send(())
        .expect("release owner after business deadline");
    let error = caller.join().expect("caller finished");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("expired before submission"));
    assert!(matches!(
        harness.writes.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
}

#[test]
fn late_completion_cannot_succeed_or_be_retried() {
    let (mut usb, harness) = setup();
    let (sent, original) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(20);
        sent.send(deadline).expect("business deadline");
        let first = usb
            .try_write_until(&[2], deadline)
            .expect_err("late receipt");
        let second = usb
            .try_write_until(&[2], Instant::now() + Duration::from_secs(1))
            .expect_err("connection locked");
        usb.close_blocking().expect("closed");
        (first, second)
    });
    let deadline = original.recv().expect("business deadline");
    let (bytes, forwarded) = harness.writes.recv().expect("submitted before expiry");
    assert_eq!(bytes, [2]);
    assert_eq!(forwarded, deadline);
    wait_until(deadline);
    harness.completion.send(Ok(())).expect("late completion");
    let (first, second) = caller.join().expect("caller finished");
    assert_eq!(first.kind(), io::ErrorKind::TimedOut);
    assert!(first.to_string().contains("completed after its deadline"));
    assert_eq!(second.kind(), io::ErrorKind::TimedOut);
    assert!(matches!(
        harness.writes.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
}

#[test]
fn long_business_deadlines_keep_the_owner_budget() {
    let (mut usb, harness) = setup();
    let original = Instant::now() + Duration::from_secs(1);
    let before = Instant::now();
    let caller = std::thread::spawn(move || {
        let result = usb.try_write_until(&[3], original);
        usb.close_blocking().expect("closed");
        result
    });
    let (_, forwarded) = harness.writes.recv().expect("submitted");
    let after = Instant::now();
    assert!(forwarded >= before + Duration::from_millis(50));
    assert!(forwarded <= after + Duration::from_millis(50));
    assert!(forwarded < original);
    harness.completion.send(Ok(())).expect("completion");
    assert_eq!(caller.join().expect("caller finished").expect("written"), 1);
}

#[test]
fn expired_before_queueing_does_not_poison_an_unwritten_connection() {
    let (mut usb, harness) = setup();
    let error = usb
        .try_write_until(&[4], Instant::now())
        .expect_err("already expired");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("expired before queueing"));
    assert!(matches!(
        harness.writes.try_recv(),
        Err(TryRecvError::Empty)
    ));
    let caller = std::thread::spawn(move || {
        let result = usb.try_write_until(&[5], Instant::now() + Duration::from_secs(1));
        usb.close_blocking().expect("closed");
        result
    });
    assert_eq!(harness.writes.recv().expect("fresh submission").0, [5]);
    harness.completion.send(Ok(())).expect("completion");
    assert_eq!(caller.join().expect("caller finished").expect("written"), 1);
}
