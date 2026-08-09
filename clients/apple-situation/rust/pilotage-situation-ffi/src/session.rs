//! Retained immutable domain records for one display session.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use airmass_core::{WeatherSnapshotEnvelope, WeatherSnapshotRecord};
use surveillance_core::{TrackDelta, TrackSnapshotHandle};

use crate::{DisplayBatch, FfiError};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct SessionState {
    tracks: BTreeMap<(u64, u64), TrackSnapshotHandle>,
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
        {
            let mut state = self.lock_state()?;
            apply_track_delta(&mut state, delta)?;
        }
        self.current_display()
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
        {
            let mut state = self.lock_state()?;
            apply_weather(&mut state, envelope);
        }
        self.current_display()
    }

    /// Get display values for the newest accepted records.
    pub fn current_display(&self) -> Result<DisplayBatch, FfiError> {
        let (tracks, weather) = {
            let state = self.lock_state()?;
            (
                state.tracks.values().cloned().collect::<Vec<_>>(),
                state.weather.clone(),
            )
        };
        pilotage_presentation::PresentationAdapter::new()
            .adapt(&tracks, weather.as_ref())
            .map(Into::into)
            .map_err(|source| FfiError::Presentation {
                message: source.to_string(),
            })
    }
}

impl PresentationSession {
    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, FfiError> {
        self.state.lock().map_err(|source| FfiError::SessionState {
            message: source.to_string(),
        })
    }
}

fn apply_track_delta(state: &mut SessionState, delta: TrackDelta) -> Result<(), FfiError> {
    match delta {
        TrackDelta::Created(handle)
        | TrackDelta::Updated(handle)
        | TrackDelta::Coasting(handle) => apply_track_handle(state, handle),
        TrackDelta::Removed {
            producer_instance_id,
            snapshot_revision,
            id,
            ..
        } => {
            let key = (producer_instance_id.get(), id.get());
            let should_remove = state
                .tracks
                .get(&key)
                .is_some_and(|current| snapshot_revision > current.snapshot_revision());
            if should_remove {
                state.tracks.remove(&key);
            }
        }
        _ => return Err(FfiError::UnsupportedTrackDelta),
    }
    Ok(())
}

fn apply_track_handle(state: &mut SessionState, handle: TrackSnapshotHandle) {
    let key = (handle.producer_instance_id().get(), handle.id.get());
    let is_newer = state
        .tracks
        .get(&key)
        .is_none_or(|current| handle.snapshot_revision() > current.snapshot_revision());
    if is_newer {
        state.tracks.insert(key, handle);
    }
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
