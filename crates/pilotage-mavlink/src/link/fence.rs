//! Revocation serializes with the final nonblocking transport write.
use std::sync::{Arc, Mutex};

use pilotage_adapter_api::SourceIncarnation;

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
    requirement: CameraWriteRequirement,
}

/// Camera discovery and source identity captured when a command is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraWriteRequirement {
    /// Camera component and device identifiers.
    pub key: (u8, u8),
    /// Camera discovery epoch.
    pub epoch: u64,
    /// Revision of the camera operations and feedback support.
    pub revision: u64,
    /// Selected link source identifier.
    pub source_id: u64,
    /// Selected link lifetime.
    pub source_incarnation: SourceIncarnation,
    /// Selected source acquisition epoch.
    pub source_epoch: u32,
}

impl CameraGuard {
    fn write(&self, operation: impl FnOnce() -> std::io::Result<bool>) -> std::io::Result<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("camera state lock poisoned"))?;
        let expected = self.requirement;
        if state.source_id != expected.source_id
            || state.source_incarnation != expected.source_incarnation
            || state.source_epoch != expected.source_epoch
        {
            return Ok(false);
        }
        let Some(report) = state.fc_reports.cameras.get(&expected.key) else {
            return Ok(false);
        };
        let fresh = report
            .information_received_at
            .and_then(|received| std::time::Instant::now().checked_duration_since(received))
            .is_some_and(|age| age < std::time::Duration::from_secs(6));
        if !report.available
            || report.epoch != expected.epoch
            || report.capability_revision != expected.revision
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
    /// Remove only this sender's ownership; an unclaimed manager is also safe.
    Release,
}

#[derive(Debug, Clone)]
struct ManagerGuard {
    state: Arc<Mutex<super::LinkState>>,
    key: (u8, u8),
    epoch: u64,
    revision: u64,
    requirement: ManagerWriteRequirement,
}

impl ManagerGuard {
    fn write(&self, operation: impl FnOnce() -> std::io::Result<bool>) -> std::io::Result<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("manager state lock poisoned"))?;
        let Some(manager) = state.gimbals.managers.get(&self.key) else {
            return Ok(false);
        };
        if manager.information.is_none()
            || manager.epoch != self.epoch
            || manager.capability_revision != self.revision
        {
            return Ok(false);
        }
        let Some((report, received)) = manager.status else {
            return Ok(false);
        };
        if self.key.1 == 0 || report.device_id != self.key.1 {
            return Ok(false);
        }
        let owner = (report.primary_system, report.primary_component);
        let fresh = std::time::Instant::now()
            .checked_duration_since(received)
            .is_some_and(|age| age < std::time::Duration::from_secs(1));
        let primary = owner == (crate::codec::GCS_SYSTEM_ID, crate::codec::GCS_COMPONENT_ID);
        let allowed = match self.requirement {
            ManagerWriteRequirement::Unclaimed => fresh && owner == (0, 0),
            ManagerWriteRequirement::Primary => fresh && primary,
            ManagerWriteRequirement::Stop => primary,
            ManagerWriteRequirement::Release => primary || owner == (0, 0),
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
        key: (u8, u8),
        epoch: u64,
        revision: u64,
        requirement: ManagerWriteRequirement,
    ) -> Self {
        Self {
            valid: self.valid.clone(),
            manager: Some(ManagerGuard {
                state,
                key,
                epoch,
                revision,
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
        requirement: CameraWriteRequirement,
    ) -> Self {
        Self {
            valid: self.valid.clone(),
            manager: None,
            camera: Some(CameraGuard { state, requirement }),
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
