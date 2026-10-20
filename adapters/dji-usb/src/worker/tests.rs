use super::*;
use crate::MacUsb;
use std::sync::mpsc;
use std::time::Duration;

mod deadline;

struct FakeDevice {
    input: Receiver<Vec<u8>>,
    polled: SyncSender<()>,
    poll_gate: Option<Receiver<()>>,
    writes: SyncSender<(Vec<u8>, Instant)>,
    completion: Receiver<Result<(), Failure>>,
    closed: SyncSender<()>,
}

impl Device for FakeDevice {
    fn poll(&mut self) -> Result<Option<Vec<u8>>, Failure> {
        self.polled.try_send(()).ok();
        if let Some(gate) = self.poll_gate.take() {
            gate.recv().expect("poll released");
        }
        match self.input.recv_timeout(Duration::from_millis(1)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn write(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Failure> {
        self.writes
            .send((bytes.to_vec(), deadline))
            .expect("write observed");
        self.completion.recv().expect("write completed")
    }

    fn close(self) -> Result<(), UsbError> {
        self.closed.send(()).ok();
        Ok(())
    }
}

struct Harness {
    input: SyncSender<Vec<u8>>,
    polled: Receiver<()>,
    writes: Receiver<(Vec<u8>, Instant)>,
    completion: SyncSender<Result<(), Failure>>,
    closed: Receiver<()>,
}

fn setup() -> (MacUsb, Harness) {
    setup_with_gate(None)
}

fn setup_with_gate(poll_gate: Option<Receiver<()>>) -> (MacUsb, Harness) {
    let (input, received) = mpsc::sync_channel(128);
    let (polled_tx, polled) = mpsc::sync_channel(1);
    let (write_tx, writes) = mpsc::sync_channel(1);
    let (completion, receipt) = mpsc::sync_channel(1);
    let (closed_tx, closed) = mpsc::sync_channel(1);
    let device = FakeDevice {
        input: received,
        polled: polled_tx,
        poll_gate,
        writes: write_tx,
        completion: receipt,
        closed: closed_tx,
    };
    let usb = MacUsb::start(move || Ok(device)).expect("owner started");
    (
        usb,
        Harness {
            input,
            polled,
            writes,
            completion,
            closed,
        },
    )
}

#[test]
fn only_actual_write_completion_returns_bytes() {
    let (mut usb, harness) = setup();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        result_tx
            .send(usb.try_write_until(&[1, 2, 3], Instant::now() + Duration::from_secs(1)))
            .expect("result");
        usb.close_blocking().expect("cleanup");
    });
    assert_eq!(harness.writes.recv().expect("submitted").0, [1, 2, 3]);
    assert!(matches!(result_rx.try_recv(), Err(TryRecvError::Empty)));
    harness.completion.send(Ok(())).expect("completion");
    assert_eq!(result_rx.recv().expect("result").expect("written"), 3);
    caller.join().expect("caller finished");
    harness.closed.recv().expect("native close");
}

#[test]
fn uncertain_write_is_never_retried() {
    let (mut usb, harness) = setup();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        let first = usb
            .try_write_until(&[4, 5], deadline)
            .expect_err("unknown write");
        let second = usb.try_write_until(&[4, 5], deadline).expect_err("latched");
        result_tx
            .send((first.kind(), second.kind()))
            .expect("result");
        usb.close_blocking().expect("cleanup");
    });
    assert_eq!(harness.writes.recv().expect("submitted").0, [4, 5]);
    harness
        .completion
        .send(Err(Failure::new(
            io::ErrorKind::TimedOut,
            "partial transfer is unknown",
        )))
        .expect("error receipt");
    assert_eq!(
        result_rx.recv().expect("result"),
        (io::ErrorKind::TimedOut, io::ErrorKind::TimedOut)
    );
    caller.join().expect("caller finished");
    assert!(matches!(
        harness.writes.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
}

#[test]
fn owner_timeout_latches_before_a_late_write_result() {
    let (mut usb, harness) = setup();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        let first = usb
            .try_write_until(&[6], deadline)
            .expect_err("owner did not answer");
        let second = usb
            .try_write_until(&[6], deadline)
            .expect_err("no second submission");
        result_tx
            .send((first.kind(), second.kind()))
            .expect("result");
    });
    assert_eq!(harness.writes.recv().expect("submitted").0, [6]);
    assert_eq!(
        result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("bounded caller wait"),
        (io::ErrorKind::TimedOut, io::ErrorKind::TimedOut)
    );
    harness.completion.send(Ok(())).expect("late result");
    harness
        .closed
        .recv_timeout(Duration::from_secs(1))
        .expect("owner closes");
    caller.join().expect("caller finished");
    assert!(matches!(
        harness.writes.try_recv(),
        Err(TryRecvError::Disconnected)
    ));
}

#[test]
fn receive_overflow_latches_instead_of_hiding_byte_loss() {
    let (mut usb, harness) = setup();
    for _ in 0..=RECEIVE_CHUNKS {
        harness.input.send(vec![1]).expect("input delivered");
    }
    harness
        .closed
        .recv_timeout(Duration::from_secs(1))
        .expect("overflow closes owner");
    let error = usb.try_read(&mut [0; 1]).expect_err("lost continuity");
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(
        usb.try_write_until(&[1], Instant::now() + Duration::from_secs(1))
            .is_err()
    );
    usb.close_blocking().expect("cleanup recorded");
}

#[test]
fn idle_input_is_nonblocking_and_drop_closes_the_owner() {
    let (mut usb, harness) = setup();
    harness.polled.recv().expect("owner polling");
    assert_eq!(
        usb.try_read(&mut [0; 10]).expect_err("idle").kind(),
        io::ErrorKind::WouldBlock
    );
    drop(usb);
    harness
        .closed
        .recv_timeout(Duration::from_secs(1))
        .expect("drop closes owner");
}

#[test]
fn expired_queued_write_never_calls_the_device() {
    let (writes, commands) = mpsc::sync_channel(1);
    let (input_tx, _input) = mpsc::sync_channel(1);
    let (started_tx, started) = mpsc::sync_channel(1);
    let (finished_tx, finished) = mpsc::sync_channel(1);
    let (seen_tx, seen) = mpsc::sync_channel(1);
    let (_complete_tx, complete) = mpsc::sync_channel(1);
    let (_input_tx, input_rx) = mpsc::sync_channel(1);
    let (closed_tx, _closed) = mpsc::sync_channel(1);
    let (polled_tx, _polled) = mpsc::sync_channel(1);
    let (receipt_tx, receipt) = mpsc::sync_channel(1);
    writes
        .send(WriteRequest {
            bytes: vec![1],
            deadline: Instant::now(),
            receipt: receipt_tx,
        })
        .expect("queued");
    let ports = Ports {
        writes: commands,
        input: input_tx,
        started: started_tx,
        finished: finished_tx,
        status: Status::new(),
    };
    run(
        || {
            Ok(FakeDevice {
                input: input_rx,
                polled: polled_tx,
                poll_gate: None,
                writes: seen_tx,
                completion: complete,
                closed: closed_tx,
            })
        },
        ports,
    );
    started.recv().expect("started").expect("opened");
    assert_eq!(
        receipt.recv().expect("receipt").expect_err("expired").kind,
        io::ErrorKind::TimedOut
    );
    assert!(matches!(seen.try_recv(), Err(TryRecvError::Disconnected)));
    finished.recv().expect("finished").expect("closed");
}

#[test]
fn stopped_owner_discards_a_queued_write_before_native_submission() {
    let (writes, commands) = mpsc::sync_channel(1);
    let (input_tx, _input) = mpsc::sync_channel(1);
    let (started_tx, _started) = mpsc::sync_channel(1);
    let (finished_tx, finished) = mpsc::sync_channel(1);
    let (seen_tx, seen) = mpsc::sync_channel(1);
    let (_complete_tx, complete) = mpsc::sync_channel(1);
    let (_input_tx, input_rx) = mpsc::sync_channel(1);
    let (closed_tx, _closed) = mpsc::sync_channel(1);
    let (polled_tx, _polled) = mpsc::sync_channel(1);
    let (receipt_tx, receipt) = mpsc::sync_channel(1);
    writes
        .send(WriteRequest {
            bytes: vec![1],
            deadline: Instant::now() + Duration::from_secs(1),
            receipt: receipt_tx,
        })
        .expect("queued");
    let status = Status::new();
    status.stop();
    run(
        || {
            Ok(FakeDevice {
                input: input_rx,
                polled: polled_tx,
                poll_gate: None,
                writes: seen_tx,
                completion: complete,
                closed: closed_tx,
            })
        },
        Ports {
            writes: commands,
            input: input_tx,
            started: started_tx,
            finished: finished_tx,
            status,
        },
    );
    assert!(receipt.recv().is_err());
    assert!(matches!(seen.try_recv(), Err(TryRecvError::Disconnected)));
    finished.recv().expect("finished").expect("closed");
}

#[test]
fn failed_startup_does_not_claim_confirmed_cleanup() {
    let (_writes, commands) = mpsc::sync_channel(1);
    let (input, _received) = mpsc::sync_channel(1);
    let (started_tx, started) = mpsc::sync_channel(1);
    let (finished_tx, finished) = mpsc::sync_channel(1);
    run::<FakeDevice>(
        || {
            Err(UsbError::Native {
                stage: 16,
                status: 1,
            })
        },
        Ports {
            writes: commands,
            input,
            started: started_tx,
            finished: finished_tx,
            status: Status::new(),
        },
    );
    assert!(matches!(
        started.recv(),
        Ok(Err(UsbError::Native { stage: 16, .. }))
    ));
    assert!(matches!(finished.recv(), Ok(Err(UsbError::CleanupUnknown))));
}
