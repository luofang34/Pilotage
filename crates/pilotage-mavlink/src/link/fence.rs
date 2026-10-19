//! Revocation serializes with the final nonblocking transport write.
use std::sync::{Arc, Mutex};

/// Shared validity of one local command generation.
#[derive(Debug, Clone)]
pub struct SendFence {
    valid: Arc<Mutex<bool>>,
    manager: Option<ManagerGuard>,
    camera: Option<CameraGuard>,
}

#[derive(Debug, Clone)]
struct CameraGuard {
    state: Arc<Mutex<super::LinkState>>,
    key: (u8, u8),
    epoch: u64,
    revision: u64,
}

impl CameraGuard {
    fn write(&self, operation: impl FnOnce() -> std::io::Result<bool>) -> std::io::Result<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("camera state lock poisoned"))?;
        let Some(report) = state.fc_reports.cameras.get(&self.key) else {
            return Ok(false);
        };
        let fresh = report
            .information_received_at
            .and_then(|received| std::time::Instant::now().checked_duration_since(received))
            .is_some_and(|age| age < std::time::Duration::from_secs(6));
        if !report.available
            || report.epoch != self.epoch
            || report.capability_revision != self.revision
            || !fresh
        {
            return Ok(false);
        }
        operation()
    }
}

/// Manager ownership required at the exact nonblocking write boundary.
#[derive(Debug, Clone, Copy)]
pub enum ManagerWriteRequirement {
    /// A fresh unclaimed manager may receive this client's claim.
    Unclaimed,
    /// A fresh manager report must identify this GCS as primary.
    Primary,
    /// A best-effort stop may outlive status freshness but cannot control another owner.
    Stop,
}

#[derive(Debug, Clone)]
struct ManagerGuard {
    state: Arc<Mutex<super::LinkState>>,
    device: u8,
    requirement: ManagerWriteRequirement,
}

impl ManagerGuard {
    fn write(&self, operation: impl FnOnce() -> std::io::Result<bool>) -> std::io::Result<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("manager state lock poisoned"))?;
        let Some((report, received)) = state.fc_reports.gimbal_manager else {
            return Ok(false);
        };
        if self.device == 0 || report.device_id != self.device {
            return Ok(false);
        }
        let owner = (report.primary_system, report.primary_component);
        let fresh = received.elapsed() < std::time::Duration::from_secs(1);
        let primary = owner == (crate::codec::GCS_SYSTEM_ID, crate::codec::GCS_COMPONENT_ID);
        let allowed = match self.requirement {
            ManagerWriteRequirement::Unclaimed => fresh && owner == (0, 0),
            ManagerWriteRequirement::Primary => fresh && primary,
            ManagerWriteRequirement::Stop => primary,
        };
        if !allowed {
            return Ok(false);
        }
        operation()
    }
}

impl Default for SendFence {
    fn default() -> Self {
        Self {
            valid: Arc::new(Mutex::new(true)),
            manager: None,
            camera: None,
        }
    }
}
impl PartialEq for SendFence {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.valid, &other.valid)
    }
}
impl SendFence {
    /// Attach the selected manager's latest receive state to this generation's write.
    #[must_use]
    pub fn for_manager(
        &self,
        state: Arc<Mutex<super::LinkState>>,
        device: u8,
        requirement: ManagerWriteRequirement,
    ) -> Self {
        Self {
            valid: self.valid.clone(),
            manager: Some(ManagerGuard {
                state,
                device,
                requirement,
            }),
            camera: None,
        }
    }

    /// Require this camera's discovered contract to remain current at the actual write.
    #[must_use]
    pub fn for_camera(
        &self,
        state: Arc<Mutex<super::LinkState>>,
        key: (u8, u8),
        epoch: u64,
        revision: u64,
    ) -> Self {
        Self {
            valid: self.valid.clone(),
            manager: None,
            camera: Some(CameraGuard {
                state,
                key,
                epoch,
                revision,
            }),
        }
    }

    /// Permanently invalidate queued commands, waiting for an in-progress write to finish.
    pub fn invalidate(&self) {
        if let Ok(mut valid) = self.valid.lock() {
            *valid = false;
        }
    }
    /// Whether this generation remains eligible for a transport write.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.valid.lock().is_ok_and(|valid| *valid)
    }

    /// Serialize a nonblocking write with revocation; never await inside the operation.
    pub(crate) fn with_valid_write(
        &self,
        operation: impl FnOnce() -> std::io::Result<bool>,
    ) -> std::io::Result<bool> {
        let write = || {
            let valid = self
                .valid
                .lock()
                .map_err(|_| std::io::Error::other("send fence lock poisoned"))?;
            if !*valid {
                return Ok(false);
            }
            operation()
        };
        if let Some(manager) = self.manager.as_ref() {
            manager.write(write)
        } else if let Some(camera) = self.camera.as_ref() {
            camera.write(write)
        } else {
            write()
        }
    }
}
#[cfg(test)]
mod tests;
