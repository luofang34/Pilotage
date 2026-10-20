use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::UsbError;
use crate::error::Failure;
use crate::worker::{self, Device, Ports, Status, WriteRequest};

const WRITE_BUDGET: Duration = Duration::from_millis(50);
const WRITE_WAIT: Duration = Duration::from_millis(100);
const START_WAIT: Duration = Duration::from_secs(3);
const CLOSE_WAIT: Duration = Duration::from_millis(750);

/// A single USB attachment with bounded receive buffering and one I/O owner.
///
/// No automatic reconnect or write retry occurs. Create a new connection only
/// after the previous owner confirms cleanup.
pub struct MacUsb {
    writes: SyncSender<WriteRequest>,
    input: Receiver<Vec<u8>>,
    finished: Receiver<Result<(), UsbError>>,
    status: Status,
    owner: Option<JoinHandle<()>>,
    cleanup_confirmed: bool,
    pending: Vec<u8>,
    offset: usize,
}

impl MacUsb {
    /// Open the upper interface of one already enumerated DJI C5 attachment.
    ///
    /// The descriptor must match the upper iAP/logiclink profile. No device
    /// configuration, USB role, or competing owner is changed.
    ///
    /// # Errors
    /// Returns an identity, native ownership, startup, or platform error.
    pub fn open_blocking(registry_id: u64) -> Result<Self, UsbError> {
        if registry_id == 0 {
            return Err(UsbError::InvalidIdentity);
        }
        #[cfg(target_os = "macos")]
        {
            Self::start(move || crate::ffi::Native::open(registry_id))
        }
        #[cfg(not(target_os = "macos"))]
        Err(UsbError::UnsupportedPlatform)
    }

    pub(crate) fn start<D: Device + 'static>(
        open: impl FnOnce() -> Result<D, UsbError> + Send + 'static,
    ) -> Result<Self, UsbError> {
        let (writes, commands) = mpsc::sync_channel(1);
        let (input_tx, input) = mpsc::sync_channel(worker::RECEIVE_CHUNKS);
        let (started_tx, started) = mpsc::sync_channel(1);
        let (finished_tx, finished) = mpsc::sync_channel(1);
        let status = Status::new();
        let ports = Ports {
            writes: commands,
            input: input_tx,
            started: started_tx,
            finished: finished_tx,
            status: status.clone(),
        };
        let owner = std::thread::Builder::new()
            .name("dji-usb-owner".to_owned())
            .spawn(move || worker::run(open, ports))
            .map_err(UsbError::WorkerSpawn)?;
        let mut connection = Self {
            writes,
            input,
            finished,
            status,
            owner: Some(owner),
            cleanup_confirmed: false,
            pending: Vec::new(),
            offset: 0,
        };
        match started.recv_timeout(START_WAIT) {
            Ok(Ok(())) => Ok(connection),
            Ok(Err(error)) => {
                connection.status.stop();
                connection.join_finished();
                Err(error)
            }
            Err(_) => {
                connection.status.stop();
                Err(UsbError::StartupUnknown)
            }
        }
    }

    /// Copy available bytes without waiting for USB input.
    ///
    /// # Errors
    /// Returns `WouldBlock` only while an intact stream has no buffered bytes.
    /// Any read failure or overflow permanently fails the connection.
    pub fn try_read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.status.check()?;
        if output.is_empty() {
            return Ok(0);
        }
        if self.offset == self.pending.len() {
            self.pending = match self.input.try_recv() {
                Ok(bytes) => bytes,
                Err(TryRecvError::Empty) => return Err(io::ErrorKind::WouldBlock.into()),
                Err(TryRecvError::Disconnected) => return Err(io::ErrorKind::BrokenPipe.into()),
            };
            self.offset = 0;
        }
        self.status.check()?;
        let count = output.len().min(self.pending.len() - self.offset);
        output[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }

    /// Submit one write and wait at most 100 ms for its actual USB result.
    ///
    /// Success means all returned bytes completed before `send_before` and
    /// within the owner's 50 ms write budget. Queueing does not renew either.
    /// It does not establish device acknowledgement. `WouldBlock` means no
    /// request was submitted. Every error after submission locks the connection.
    ///
    /// # Errors
    /// Returns an argument, queue, transport, or unknown-outcome error.
    pub fn try_write_until(&mut self, bytes: &[u8], send_before: Instant) -> io::Result<usize> {
        self.status.check()?;
        if bytes.is_empty() || bytes.len() > worker::CHUNK_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DJI USB write length must be 1..16384",
            ));
        }
        let now = Instant::now();
        let deadline = send_before.min(now + WRITE_BUDGET);
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "DJI USB write expired before queueing",
            ));
        }
        let (receipt, completed) = mpsc::sync_channel(1);
        let request = WriteRequest {
            bytes: bytes.to_vec(),
            deadline,
            receipt,
        };
        match self.writes.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(io::ErrorKind::WouldBlock.into()),
            Err(TrySendError::Disconnected(_)) => return Err(io::ErrorKind::BrokenPipe.into()),
        }
        match completed.recv_timeout(WRITE_WAIT) {
            Ok(Ok(count)) => Ok(count),
            Ok(Err(error)) => Err(error.io()),
            Err(_) => {
                let error = Failure::new(
                    io::ErrorKind::TimedOut,
                    "DJI USB write outcome is unknown; no retry is permitted",
                );
                self.status.fail(error.clone());
                Err(error.io())
            }
        }
    }

    /// Stop input, restore the original alternate setting, and close the owner.
    ///
    /// # Errors
    /// Returns an error if cleanup is failed or remains unknown after 750 ms.
    pub fn close_blocking(mut self) -> Result<(), UsbError> {
        self.status.stop();
        let result = self.receive_cleanup_blocking();
        self.join_finished();
        result
    }

    fn receive_cleanup_blocking(&mut self) -> Result<(), UsbError> {
        let result = self
            .finished
            .recv_timeout(CLOSE_WAIT)
            .map_err(|_| UsbError::CleanupUnknown)?;
        self.cleanup_confirmed = result.is_ok();
        result
    }

    fn join_finished(&mut self) {
        if self.owner.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(owner) = self.owner.take()
            && owner.join().is_err()
        {
            tracing::error!("DJI USB owner panicked");
        }
    }
}

impl Drop for MacUsb {
    fn drop(&mut self) {
        self.status.stop();
        if !self.cleanup_confirmed && matches!(self.finished.try_recv(), Ok(Ok(()))) {
            self.cleanup_confirmed = true;
        }
        self.join_finished();
        if !self.cleanup_confirmed {
            tracing::debug!("DJI USB cleanup is not yet confirmed");
        }
    }
}

#[cfg(test)]
mod tests;
