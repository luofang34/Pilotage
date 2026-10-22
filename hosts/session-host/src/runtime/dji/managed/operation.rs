use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::DjiHostError;

const DEADLINE: Duration = Duration::from_secs(4);

pub(super) struct Operation<T> {
    result: Receiver<T>,
    worker: Option<JoinHandle<()>>,
    deadline: Instant,
}

impl<T: Send + 'static> Operation<T> {
    pub(super) fn start(run: impl FnOnce() -> T + Send + 'static) -> Result<Self, DjiHostError> {
        let (output, result) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("dji-connection-lifecycle".into())
            .spawn(move || {
                output.send(run()).ok();
            })
            .map_err(DjiHostError::LifecycleSpawn)?;
        Ok(Self {
            result,
            worker: Some(worker),
            deadline: Instant::now() + DEADLINE,
        })
    }

    pub(super) fn poll(&mut self) -> Result<Option<T>, TryRecvError> {
        match self.result.try_recv() {
            Ok(value) => Ok(Some(value)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

impl<T> Drop for Operation<T> {
    fn drop(&mut self) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(worker) = self.worker.take() {
                worker.join().ok();
            }
        } else {
            // The worker owns any late adapter and closes it if the receiver is gone.
            tracing::debug!(
                task = "dji-connection-lifecycle",
                "lifecycle worker still finishing"
            );
        }
    }
}

#[cfg(test)]
mod tests;
