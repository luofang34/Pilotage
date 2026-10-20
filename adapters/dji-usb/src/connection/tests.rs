use super::*;

struct ClosingDevice {
    close_result: Result<(), UsbError>,
}

impl Device for ClosingDevice {
    fn poll(&mut self) -> Result<Option<Vec<u8>>, Failure> {
        Ok(None)
    }

    fn write(&mut self, _bytes: &[u8], _deadline: Instant) -> Result<(), Failure> {
        Err(Failure::new(io::ErrorKind::BrokenPipe, "device is closing"))
    }

    fn close(self) -> Result<(), UsbError> {
        self.close_result
    }
}

fn closing_owner(close_result: Result<(), UsbError>) -> (MacUsb, SyncSender<()>) {
    let (writes, commands) = mpsc::sync_channel(1);
    let (input_tx, input) = mpsc::sync_channel(1);
    let (started_tx, started) = mpsc::sync_channel(1);
    let (finished_tx, finished) = mpsc::sync_channel(1);
    let (release, exit) = mpsc::sync_channel(0);
    let status = Status::new();
    let ports = Ports {
        writes: commands,
        input: input_tx,
        started: started_tx,
        finished: finished_tx,
        status: status.clone(),
    };
    let owner = std::thread::spawn(move || {
        worker::run(|| Ok(ClosingDevice { close_result }), ports);
        exit.recv().expect("owner exit permitted");
    });
    started.recv().expect("startup result").expect("opened");
    (
        MacUsb {
            writes,
            input,
            finished,
            status,
            owner: Some(owner),
            cleanup_confirmed: false,
            pending: Vec::new(),
            offset: 0,
        },
        release,
    )
}

#[test]
fn cleanup_receipt_confirms_restoration_before_the_owner_thread_exits() {
    let (mut usb, release) = closing_owner(Ok(()));
    usb.status.stop();
    usb.receive_cleanup_blocking()
        .expect("native cleanup succeeded");
    assert!(usb.cleanup_confirmed);
    assert!(!usb.owner.as_ref().expect("owner retained").is_finished());
    usb.join_finished();
    assert!(usb.owner.is_some());
    drop(usb);
    release
        .send(())
        .expect("release owner after connection drop");
}

#[test]
fn owner_exit_does_not_confirm_failed_cleanup() {
    let (mut usb, release) = closing_owner(Err(UsbError::CleanupUnknown));
    usb.status.stop();
    assert!(usb.receive_cleanup_blocking().is_err());
    assert!(!usb.cleanup_confirmed);
    release.send(()).expect("permit owner exit");
    usb.owner
        .take()
        .expect("owner retained")
        .join()
        .expect("owner exited");
    assert!(!usb.cleanup_confirmed);
}
