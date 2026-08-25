use std::process::Child;
use std::sync::mpsc;
use std::time::Duration;

use crate::AviateSupervisorError;

pub(super) struct ReapableOwner {
    child: Option<Child>,
    reaper: Option<OwnerReaper>,
}

impl ReapableOwner {
    pub(super) fn spawn(
        spawn_child: impl FnOnce() -> Result<Child, AviateSupervisorError>,
    ) -> Result<Self, AviateSupervisorError> {
        spawn_with_reaper(OwnerReaper::spawn, spawn_child)
    }

    pub(super) fn child_mut(&mut self) -> Result<&mut Child, AviateSupervisorError> {
        self.child
            .as_mut()
            .ok_or_else(|| AviateSupervisorError::protocol("the process owner is missing"))
    }

    pub(super) fn id(&self) -> Result<u32, AviateSupervisorError> {
        self.child
            .as_ref()
            .map(Child::id)
            .ok_or_else(|| AviateSupervisorError::protocol("the process owner is missing"))
    }
}

impl Drop for ReapableOwner {
    fn drop(&mut self) {
        let Some(child) = self.child.take() else {
            return;
        };
        let Some(reaper) = self.reaper.take() else {
            tracing::error!(pid = child.id(), "process-owner reaper is missing");
            reap_owner_blocking(child);
            return;
        };
        reaper.handoff(child);
    }
}

struct OwnerReaper {
    sender: mpsc::Sender<Child>,
}

impl OwnerReaper {
    fn spawn() -> Result<Self, AviateSupervisorError> {
        Self::spawn_worker(None)
    }

    fn spawn_worker(completion: Option<mpsc::Sender<()>>) -> Result<Self, AviateSupervisorError> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("aviate-owner-reaper".to_owned())
            .spawn(move || run_reaper_blocking(receiver, completion))
            .map_err(|source| AviateSupervisorError::ProcessIo {
                operation: "spawn process-owner reaper",
                source,
            })?;
        Ok(Self { sender })
    }

    fn handoff(self, child: Child) {
        if let Err(error) = self.sender.send(child) {
            let child = error.0;
            tracing::error!(pid = child.id(), "process-owner reaper channel closed");
            reap_owner_blocking(child);
        }
    }
}

fn spawn_with_reaper(
    spawn_reaper: impl FnOnce() -> Result<OwnerReaper, AviateSupervisorError>,
    spawn_child: impl FnOnce() -> Result<Child, AviateSupervisorError>,
) -> Result<ReapableOwner, AviateSupervisorError> {
    let reaper = spawn_reaper()?;
    let child = spawn_child()?;
    Ok(ReapableOwner {
        child: Some(child),
        reaper: Some(reaper),
    })
}

fn run_reaper_blocking(receiver: mpsc::Receiver<Child>, completion: Option<mpsc::Sender<()>>) {
    let Ok(child) = receiver.recv() else {
        return;
    };
    reap_owner_blocking(child);
    if let Some(completion) = completion
        && completion.send(()).is_err()
    {
        tracing::debug!("process-owner reap observer is closed");
    }
}

fn reap_owner_blocking(mut child: Child) {
    let pid = child.id();
    let mut reported_failure = false;
    loop {
        match child.wait() {
            Ok(status) => {
                tracing::debug!(pid, %status, "reaped Aviate process owner");
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                if !reported_failure {
                    tracing::error!(pid, %error, "Aviate process-owner reap failed; retrying");
                    reported_failure = true;
                }
                std::thread::park_timeout(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
#[path = "reaper/tests.rs"]
mod tests;
