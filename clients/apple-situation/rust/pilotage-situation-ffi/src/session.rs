//! Retained immutable domain records for one display session.

use std::sync::{Arc, Mutex, MutexGuard};

use airmass_core::{WeatherSnapshotEnvelope, WeatherSnapshotRecord};
use pilotage_presentation::{PointChange, PresentationAdapter};

use crate::{DisplayBatch, FfiError};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct SessionState {
    presentation: PresentationAdapter,
    weather: Option<WeatherSnapshotEnvelope>,
}

/// Retains the newest immutable producer records for the Swift host.
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
        display_for_state(&state, change)
    }

    /// Accept one versioned Airmass weather snapshot record.
    pub fn accept_weather_record(&self, record_json: String) -> Result<DisplayBatch, FfiError> {
        let record: WeatherSnapshotRecord =
            serde_json::from_str(&record_json).map_err(|source| FfiError::WeatherRecord {
                message: source.to_string(),
            })?;
        let envelope = record
            .into_envelope()
            .map_err(|source| FfiError::WeatherRecord {
                message: source.to_string(),
            })?;
        let mut state = self.lock_state()?;
        apply_weather(&mut state, envelope);
        display_for_state(&state, None)
    }

    /// Get display values for the newest accepted records.
    pub fn current_display(&self) -> Result<DisplayBatch, FfiError> {
        let state = self.lock_state()?;
        display_for_state(&state, None)
    }
}

impl PresentationSession {
    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, FfiError> {
        self.state.lock().map_err(|source| FfiError::SessionState {
            message: source.to_string(),
        })
    }
}

fn display_for_state(
    state: &SessionState,
    change: Option<PointChange>,
) -> Result<DisplayBatch, FfiError> {
    let mut batch = state
        .presentation
        .adapt(state.weather.as_ref())
        .map_err(|source| FfiError::Presentation {
            message: source.to_string(),
        })?;
    batch.point_changes.extend(change);
    Ok(batch.into())
}

fn apply_weather(state: &mut SessionState, envelope: WeatherSnapshotEnvelope) {
    let is_newer = state.weather.as_ref().is_none_or(|current| {
        current.producer_instance_id() != envelope.producer_instance_id()
            || envelope.snapshot_revision() > current.snapshot_revision()
    });
    if is_newer {
        state.weather = Some(envelope);
    }
}
