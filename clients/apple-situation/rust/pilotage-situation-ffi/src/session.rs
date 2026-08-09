//! Retained display state for one presentation session.

use std::sync::{Arc, Mutex, MutexGuard};

use pilotage_presentation::{PointChange, PresentationAdapter};

use crate::{DisplayBatch, FfiError};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct SessionState {
    presentation: PresentationAdapter,
}

/// Retains display state for the Swift host.
#[derive(uniffi::Object)]
pub struct PresentationSession {
    state: Mutex<SessionState>,
}

#[uniffi::export]
impl PresentationSession {
    /// Create an empty session.
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SessionState::default()),
        })
    }

    /// Accept one versioned Surveillance track record.
    pub fn accept_track_record(&self, record_json: String) -> Result<DisplayBatch, FfiError> {
        let record: surveillance_core::TrackRecord =
            serde_json::from_str(&record_json).map_err(|source| FfiError::TrackRecord {
                message: source.to_string(),
            })?;
        let delta = record
            .into_delta()
            .map_err(|source| FfiError::TrackRecord {
                message: source.to_string(),
            })?;
        let mut state = self.lock_state()?;
        let change = state.presentation.apply_traffic_delta(&delta);
        Ok(display_for_state(&state, change))
    }

    /// Get display values for the newest accepted records.
    pub fn current_display(&self) -> Result<DisplayBatch, FfiError> {
        let state = self.lock_state()?;
        Ok(display_for_state(&state, None))
    }
}

impl PresentationSession {
    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, FfiError> {
        self.state.lock().map_err(|source| FfiError::SessionState {
            message: source.to_string(),
        })
    }
}

fn display_for_state(state: &SessionState, change: Option<PointChange>) -> DisplayBatch {
    let mut batch = state.presentation.adapt();
    batch.point_changes.extend(change);
    batch.into()
}
