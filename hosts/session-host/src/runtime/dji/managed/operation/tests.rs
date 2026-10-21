use super::{Duration, Operation, mpsc};

#[test]
fn pending_connection_does_not_block_the_control_loop() {
    let (entered_tx, entered) = mpsc::sync_channel(1);
    let (release, waiting) = mpsc::sync_channel(1);
    let mut operation = Operation::start(move || {
        entered_tx.send(()).expect("entered");
        waiting.recv().expect("release connection work");
        42
    })
    .expect("start worker");
    entered
        .recv_timeout(Duration::from_secs(1))
        .expect("worker entered");
    assert!(operation.poll().expect("alive").is_none());
    release.send(()).expect("release");
    assert_eq!(
        operation
            .result
            .recv_timeout(Duration::from_secs(1))
            .expect("completion"),
        42
    );
}
