use super::*;
use std::time::Duration;

fn channel() -> PayloadChannel {
    let limit = turret_core::AngularRate::radians_per_second(0.8).unwrap();
    PayloadChannel::new(
        turret_mavlink::manager_capabilities(1, limit, limit).unwrap(),
        1,
        Instant::now(),
    )
}
fn timing() -> PayloadTiming {
    PayloadTiming {
        send_before: Instant::now() + Duration::from_secs(1),
        valid_until: None,
        ack_before: None,
        completion_before: None,
    }
}

#[test]
fn admitted_is_not_sent_and_revoke_fences_the_final_write() {
    let channel = channel();
    let write = channel
        .submit(PayloadCommand::Recenter, 1, timing())
        .unwrap();
    assert!(
        channel
            .snapshot_cached()
            .unwrap()
            .unwrap()
            .progress
            .sent_at
            .is_none()
    );
    channel.revoke().unwrap();
    assert!(!write.write(1, || panic!("revoked write")).unwrap());
    assert_eq!(
        channel.snapshot_cached().unwrap().unwrap().progress.ended,
        Some(turret_core::EndReason::Revoked)
    );
}

#[test]
fn pending_command_keeps_terminal_evidence_when_active_command_is_also_revoked() {
    let channel = channel();
    let first = channel
        .submit(PayloadCommand::Recenter, 1, timing())
        .unwrap();
    assert!(first.write(1, || Ok(1)).unwrap());
    let second = channel
        .submit(PayloadCommand::Recenter, 2, timing())
        .unwrap();
    channel.revoke().unwrap();
    let snapshot = channel.snapshot_cached().unwrap().unwrap();
    assert_eq!(snapshot.command.identity.id, 2);
    assert_eq!(
        snapshot.progress.ended,
        Some(turret_core::EndReason::Revoked)
    );
    assert!(!second.write(1, || panic!("revoked pending write")).unwrap());
    assert_eq!(channel.poll_effect().unwrap(), Effect::CancelAction);
}

#[test]
fn capability_update_preserves_pending_termination_and_active_cancel_effect() {
    let channel = channel();
    let first = channel
        .submit(PayloadCommand::Recenter, 1, timing())
        .unwrap();
    assert!(first.write(1, || Ok(1)).unwrap());
    channel
        .submit(PayloadCommand::Recenter, 2, timing())
        .unwrap();
    let limit = turret_core::AngularRate::radians_per_second(0.8).unwrap();
    channel
        .update_capabilities(turret_mavlink::manager_capabilities(2, limit, limit).unwrap())
        .unwrap();
    assert_eq!(
        channel.snapshot_cached().unwrap().unwrap().progress.ended,
        Some(turret_core::EndReason::Revoked)
    );
    assert_eq!(channel.poll_effect().unwrap(), Effect::CancelAction);
}

#[test]
fn write_then_submit_uses_clock_after_waiting_for_shared_executor() {
    let channel = channel();
    let first = channel
        .submit(PayloadCommand::Recenter, 1, timing())
        .unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        first
            .write(1, || {
                entered_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok(1)
            })
            .unwrap()
    });
    entered_rx.recv().unwrap();
    let second_channel = channel.clone();
    let second =
        std::thread::spawn(move || second_channel.submit(PayloadCommand::Recenter, 2, timing()));
    finish_tx.send(()).unwrap();
    assert!(writer.join().unwrap());
    assert!(second.join().unwrap().is_ok());
}

#[test]
fn final_write_checks_original_deadline_and_full_write_evidence() {
    let channel = channel();
    let write = channel
        .submit(PayloadCommand::Recenter, 1, timing())
        .unwrap();
    assert!(write.write(2, || Ok(1)).is_err());
    let snapshot = channel.snapshot_cached().unwrap().unwrap();
    assert!(snapshot.progress.sent_at.is_none());
    assert_eq!(
        snapshot.progress.ended,
        Some(turret_core::EndReason::TransportFailure)
    );
    assert!(
        channel
            .submit(
                PayloadCommand::Recenter,
                2,
                PayloadTiming {
                    send_before: Instant::now(),
                    ..timing()
                }
            )
            .is_err()
    );
}
