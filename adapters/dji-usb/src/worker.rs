use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::UsbError;
use crate::error::Failure;

pub(crate) const CHUNK_BYTES: usize = 16 * 1024;
pub(crate) const RECEIVE_CHUNKS: usize = 64;

pub(crate) trait Device {
    fn poll(&mut self) -> Result<Option<Vec<u8>>, Failure>;
    fn write(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Failure>;
    fn close(self) -> Result<(), UsbError>;
}

pub(crate) struct WriteRequest {
    pub(crate) bytes: Vec<u8>,
    pub(crate) deadline: Instant,
    pub(crate) receipt: SyncSender<Result<usize, Failure>>,
}

#[derive(Clone)]
pub(crate) struct Status {
    stopping: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<Failure>>>,
}

impl Status {
    pub(crate) fn new() -> Self {
        Self {
            stopping: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(crate) fn fail(&self, failure: Failure) {
        self.stop();
        if let Ok(mut slot) = self.failure.lock()
            && slot.is_none()
        {
            tracing::error!(%failure, "DJI USB connection failed");
            *slot = Some(failure);
        }
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        let slot = self
            .failure
            .lock()
            .map_err(|_| io::Error::other("DJI USB failure state is unavailable"))?;
        if let Some(failure) = slot.as_ref() {
            Err(failure.io())
        } else if self.stopped() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "DJI USB owner stopped",
            ))
        } else {
            Ok(())
        }
    }
}

pub(crate) struct Ports {
    pub(crate) writes: Receiver<WriteRequest>,
    pub(crate) input: SyncSender<Vec<u8>>,
    pub(crate) started: SyncSender<Result<(), UsbError>>,
    pub(crate) finished: SyncSender<Result<(), UsbError>>,
    pub(crate) status: Status,
}

pub(crate) fn run<D: Device>(open: impl FnOnce() -> Result<D, UsbError>, ports: Ports) {
    let mut device = match open() {
        Ok(device) => device,
        Err(error) => {
            ports.status.stop();
            ports.started.send(Err(error)).ok();
            ports.finished.send(Err(UsbError::CleanupUnknown)).ok();
            return;
        }
    };
    if ports.started.send(Ok(())).is_err() {
        ports.status.stop();
    }
    while !ports.status.stopped() {
        match ports.writes.try_recv() {
            Ok(request) => write(&mut device, request, &ports.status),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ports.status.stop(),
        }
        if !ports.status.stopped() {
            match device.poll() {
                Ok(Some(bytes)) => receive(bytes, &ports.input, &ports.status),
                Ok(None) => {}
                Err(error) => ports.status.fail(error),
            }
        }
    }
    let result = device.close();
    if let Err(error) = &result {
        tracing::error!(%error, "DJI USB owner could not confirm cleanup");
    }
    ports.finished.send(result).ok();
}

fn write(device: &mut impl Device, request: WriteRequest, status: &Status) {
    let result = if Instant::now() >= request.deadline || status.stopped() {
        Err(Failure::new(
            io::ErrorKind::TimedOut,
            "DJI USB write expired before submission",
        ))
    } else {
        device
            .write(&request.bytes, request.deadline)
            .and_then(|()| {
                if Instant::now() >= request.deadline {
                    Err(Failure::new(
                        io::ErrorKind::TimedOut,
                        "DJI USB write completed after its deadline",
                    ))
                } else {
                    Ok(request.bytes.len())
                }
            })
    };
    if let Err(error) = &result {
        status.fail(error.clone());
    }
    request.receipt.send(result).ok();
}

fn receive(bytes: Vec<u8>, input: &SyncSender<Vec<u8>>, status: &Status) {
    if bytes.is_empty() {
        return;
    }
    if bytes.len() > CHUNK_BYTES {
        status.fail(Failure::new(
            io::ErrorKind::InvalidData,
            "DJI USB read exceeded its buffer",
        ));
        return;
    }
    match input.try_send(bytes) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => status.fail(Failure::new(
            io::ErrorKind::OutOfMemory,
            "DJI USB receive queue overflowed; byte continuity is lost",
        )),
        Err(TrySendError::Disconnected(_)) => status.stop(),
    }
}

#[cfg(test)]
mod tests;
