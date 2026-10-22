use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::UsbError;

/// Registry notifications with optional native connection access on one owner task.
///
/// Notifications coalesce. A notification requires a new snapshot; it does not
/// establish that a particular data device is safe to open.
pub struct UsbEvents {
    changed: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    worker: Option<JoinHandle<()>>,
    permitted: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    heartbeat: Arc<AtomicU64>,
    origin: Instant,
}

impl UsbEvents {
    /// Subscribe to device and interface events without opening USB handles.
    ///
    /// # Errors
    /// Returns an error if registry subscription cannot complete within one second.
    pub fn subscribe_blocking() -> Result<Self, UsbError> {
        Self::start_blocking(None)
    }

    /// Subscribe and prepare the native upper-port handshake on a dedicated task.
    ///
    /// `ground_recovery` permits one software role cycle for a missed handshake.
    /// The caller must confirm that interrupting this connection is safe on the ground.
    /// It applies only to the startup connection, expires after five seconds,
    /// and never renews on reconnect.
    ///
    /// # Errors
    /// Returns an error if another host owns access or subscription fails.
    pub fn subscribe_access_blocking(ground_recovery: bool) -> Result<Self, UsbError> {
        Self::start_blocking(Some(ground_recovery))
    }

    fn start_blocking(access: Option<bool>) -> Result<Self, UsbError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (changed_tx, changed) = mpsc::sync_channel(1);
        let (done_tx, done) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let permitted = Arc::new(AtomicBool::new(false));
        let admission = Arc::clone(&permitted);
        let status = Arc::new(AtomicU8::new(0));
        let published = Arc::clone(&status);
        let heartbeat = Arc::new(AtomicU64::new(0));
        let pulse = Arc::clone(&heartbeat);
        let origin = Instant::now();
        let worker = thread::Builder::new()
            .name("dji-usb-events".into())
            .spawn(move || {
                match crate::ffi::RegistryWatch::open(access) {
                    Ok(mut watch) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            while !stopping.load(Ordering::Acquire) {
                                watch.permit(admission.load(Ordering::Acquire));
                                let changed = watch.changed();
                                pulse.store(origin.elapsed().as_millis() as u64, Ordering::Release);
                                let status = watch.status();
                                if published.swap(status, Ordering::AcqRel) != status {
                                    tracing::info!(
                                        state = ?crate::UsbAccessStatus::from_native(status),
                                        "DJI USB access state changed"
                                    );
                                }
                                if changed {
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
            permitted,
            status,
            heartbeat,
            origin,
        };
        ready_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| UsbError::StartupUnknown)??;
        Ok(events)
    }

    /// Admit handshake work only when the data owner and its cleanup have finished.
    pub fn permit_handshake(&self, permitted: bool) {
        self.permitted.store(permitted, Ordering::Release);
    }

    /// Return the access task's last published state, independent of aircraft state.
    #[must_use]
    pub fn access_status_cached(&self) -> crate::UsbAccessStatus {
        let pulse = self.heartbeat.load(Ordering::Acquire);
        if self
            .origin
            .elapsed()
            .saturating_sub(Duration::from_millis(pulse))
            > Duration::from_secs(15)
        {
            self.stop.store(true, Ordering::Release);
            return crate::UsbAccessStatus::Blocked;
        }
        crate::UsbAccessStatus::from_native(self.status.load(Ordering::Acquire))
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
