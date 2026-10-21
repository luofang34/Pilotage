use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::UsbError;

/// Read-only registry notifications, armed before the caller reads a snapshot.
///
/// Notifications coalesce. A notification means that the caller must read a new
/// snapshot, not that a particular device is safe to open. No USB handle is opened.
pub struct UsbEvents {
    changed: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl UsbEvents {
    /// Subscribe to device and interface arrival and termination.
    ///
    /// # Errors
    /// Returns an error if registry subscription cannot complete within one second.
    pub fn subscribe_blocking() -> Result<Self, UsbError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (changed_tx, changed) = mpsc::sync_channel(1);
        let (done_tx, done) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("dji-usb-events".into())
            .spawn(move || {
                match crate::ffi::RegistryWatch::open() {
                    Ok(mut watch) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            while !stopping.load(Ordering::Acquire) {
                                if watch.changed() {
                                    match changed_tx.try_send(()) {
                                        Ok(()) | Err(mpsc::TrySendError::Full(())) => {}
                                        Err(mpsc::TrySendError::Disconnected(())) => break,
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        ready_tx.send(Err(error)).ok();
                    }
                }
                done_tx.send(()).ok();
            })
            .map_err(UsbError::WorkerSpawn)?;
        let events = Self {
            changed,
            stop,
            done,
            worker: Some(worker),
        };
        ready_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| UsbError::StartupUnknown)??;
        Ok(events)
    }

    /// Consume coalesced notifications without waiting.
    ///
    /// # Errors
    /// Returns an error when the registry observer has stopped.
    pub fn changed(&self) -> Result<bool, UsbError> {
        match self.changed.try_recv() {
            Ok(()) => Ok(true),
            Err(mpsc::TryRecvError::Empty) => Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => Err(UsbError::WorkerEnded),
        }
    }
}

impl Drop for UsbEvents {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self.done.recv_timeout(Duration::from_millis(500)).is_ok() {
            if let Some(worker) = self.worker.take() {
                worker.join().ok();
            }
        } else {
            tracing::error!(
                task = "dji-usb-events",
                "registry observer shutdown unconfirmed"
            );
        }
    }
}

#[cfg(test)]
mod tests;
