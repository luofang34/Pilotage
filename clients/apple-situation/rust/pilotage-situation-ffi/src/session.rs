//! Retained display state for one presentation session.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use airmass_core::{
    WeatherSnapshotEnvelope, WeatherSnapshotRecord, WeatherSnapshotRecordHeader, WeatherStationId,
};
use airmass_geojson::{Wgs84Position, map_snapshot_transition};
use pilotage_presentation::{Coordinate, PointChange, PresentationAdapter};
use pilotage_terrain_query::TerrainArchive;
use surveillance_core::{TrackRecord, TrackRecordHeader};

use crate::WeatherStationPosition;
use crate::{DisplayBatch, FfiError, PresentationRadioState, PresentationSourceObservation};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct SessionState {
    presentation: PresentationAdapter,
    terrain: Option<TerrainArchive>,
    weather_snapshot: Option<WeatherSnapshotEnvelope>,
    weather_positions: BTreeMap<String, Wgs84Position>,
    source_observation: PresentationSourceObservation,
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
    pub fn accept_track_record(
        &self,
        record_json: String,
        now_micros: u64,
    ) -> Result<DisplayBatch, FfiError> {
        validate_track_header(&record_json)?;
        let record: TrackRecord = serde_json::from_str(&record_json).map_err(track_record_error)?;
        let delta = record.into_delta().map_err(track_record_error)?;
        let mut state = self.lock_state()?;
        state.presentation.advance_time(now_micros);
        let SessionState {
            presentation,
            terrain,
            ..
        } = &mut *state;
        let change = presentation
            .apply_traffic_delta_with_terrain_blocking(&delta, |coordinate| {
                terrain_elevation_blocking(terrain, coordinate)
            })?;
        Ok(display_for_state(&state, change.into_iter().collect()))
    }

    /// Accept one versioned Airmass weather snapshot record.
    pub fn accept_weather_record(
        &self,
        record_json: String,
        now_micros: u64,
    ) -> Result<DisplayBatch, FfiError> {
        validate_weather_header(&record_json)?;
        let record: WeatherSnapshotRecord =
            serde_json::from_str(&record_json).map_err(weather_record_error)?;
        let current = record.into_envelope().map_err(weather_record_error)?;
        let mut state = self.lock_state()?;
        state.presentation.advance_time(now_micros);
        if weather_record_is_stale(state.weather_snapshot.as_ref(), &current) {
            return Ok(display_for_state(&state, Vec::new()));
        }
        let deltas = map_weather_transition(&state, &current);
        let changes = apply_weather_deltas(&mut state, deltas)?;
        state.weather_snapshot = Some(current);
        Ok(display_for_state(&state, changes))
    }

    /// Take the weather station positions from one published navigation-data cycle.
    ///
    /// A text weather report names its station and carries no position, so a client with
    /// no cycle draws no weather however well the report decoded. The caller supplies the
    /// encoded cycle, because where a cycle comes from is a delivery question and this
    /// session only needs its contents.
    pub fn load_weather_stations_from_cycle(
        &self,
        cycle_bytes: Vec<u8>,
    ) -> Result<DisplayBatch, FfiError> {
        let snapshot =
            pilotage_navdata_cycle::load_cycle_bytes("cycle", &cycle_bytes).map_err(|source| {
                FfiError::WeatherStationPosition {
                    message: source.to_string(),
                }
            })?;
        let positions = pilotage_navdata_cycle::weather_station_positions(&snapshot)
            .into_iter()
            .map(|station| WeatherStationPosition {
                station_id: station.station_id,
                latitude_deg: station.latitude_deg,
                longitude_deg: station.longitude_deg,
            })
            .collect();
        self.replace_weather_station_positions(positions)
    }

    /// Replace the navigation-data positions used for weather stations.
    pub fn replace_weather_station_positions(
        &self,
        positions: Vec<WeatherStationPosition>,
    ) -> Result<DisplayBatch, FfiError> {
        let catalog = weather_position_catalog(positions)?;
        let mut state = self.lock_state()?;
        let mut changes = state.presentation.clear_weather();
        state.weather_positions = catalog;
        apply_source_observation(&mut state);
        if let Some(current) = state.weather_snapshot.clone() {
            let deltas = map_weather_initial(&state, &current);
            changes.extend(apply_weather_deltas(&mut state, deltas)?);
        }
        Ok(display_for_state(&state, changes))
    }

    /// Clear retained traffic and weather display state.
    pub fn clear_radio_records(&self) -> Result<DisplayBatch, FfiError> {
        let mut state = self.lock_state()?;
        state.presentation.clear_radio_state();
        state.weather_snapshot = None;
        state.source_observation.radio_state = PresentationRadioState::Suspended;
        state.source_observation.radio_receivers.clear();
        apply_source_observation(&mut state);
        Ok(display_for_state(&state, Vec::new()))
    }

    /// Open the Terrarium archive used for vertical placement.
    ///
    /// Call this function before the session accepts traffic or weather records.
    pub fn load_terrain_archive_blocking(&self, archive_path: String) -> Result<(), FfiError> {
        let terrain = TerrainArchive::open_blocking(&archive_path).map_err(|source| {
            FfiError::TerrainArchive {
                message: source.to_string(),
            }
        })?;
        self.lock_state()?.terrain = Some(terrain);
        Ok(())
    }

    /// Record source facts and advance the display clock.
    pub fn observe_sources(
        &self,
        observation: PresentationSourceObservation,
        now_micros: u64,
    ) -> Result<DisplayBatch, FfiError> {
        let mut state = self.lock_state()?;
        state.source_observation = observation;
        state.presentation.advance_time(now_micros);
        apply_source_observation(&mut state);
        Ok(display_for_state(&state, Vec::new()))
    }

    /// Set one application layer control.
    pub fn set_layer_enabled(
        &self,
        layer_id: String,
        enabled: bool,
    ) -> Result<DisplayBatch, FfiError> {
        let mut state = self.lock_state()?;
        if !state.presentation.set_layer_enabled(&layer_id, enabled) {
            return Err(FfiError::UnknownLayer { layer_id });
        }
        Ok(display_for_state(&state, Vec::new()))
    }

    /// Get display values for the newest accepted records.
    pub fn current_display(&self, now_micros: u64) -> Result<DisplayBatch, FfiError> {
        let mut state = self.lock_state()?;
        state.presentation.advance_time(now_micros);
        Ok(display_for_state(&state, Vec::new()))
    }
}

fn apply_source_observation(state: &mut SessionState) {
    let weather_positions_available = !state.weather_positions.is_empty();
    state.presentation.observe_sources(
        state
            .source_observation
            .clone()
            .into_portable(weather_positions_available),
    );
}

impl PresentationSession {
    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, FfiError> {
        self.state.lock().map_err(|source| FfiError::SessionState {
            message: source.to_string(),
        })
    }
}

fn validate_track_header(record_json: &str) -> Result<(), FfiError> {
    let header: TrackRecordHeader =
        serde_json::from_str(record_json).map_err(track_record_error)?;
    header.validate().map_err(track_record_error)
}

fn validate_weather_header(record_json: &str) -> Result<(), FfiError> {
    let header: WeatherSnapshotRecordHeader =
        serde_json::from_str(record_json).map_err(weather_record_error)?;
    header.validate().map_err(weather_record_error)
}

fn weather_record_is_stale(
    previous: Option<&WeatherSnapshotEnvelope>,
    current: &WeatherSnapshotEnvelope,
) -> bool {
    previous.is_some_and(|prior| {
        prior.producer_instance_id() == current.producer_instance_id()
            && prior.snapshot_revision().get() >= current.snapshot_revision().get()
    })
}

fn map_weather_transition(
    state: &SessionState,
    current: &WeatherSnapshotEnvelope,
) -> Vec<airmass_geojson::FeatureDelta> {
    map_snapshot_transition(
        state.weather_snapshot.as_ref(),
        current,
        &|station: &WeatherStationId| state.weather_positions.get(station.as_str()).copied(),
    )
}

fn map_weather_initial(
    state: &SessionState,
    current: &WeatherSnapshotEnvelope,
) -> Vec<airmass_geojson::FeatureDelta> {
    map_snapshot_transition(None, current, &|station: &WeatherStationId| {
        state.weather_positions.get(station.as_str()).copied()
    })
}

fn apply_weather_deltas(
    state: &mut SessionState,
    deltas: Vec<airmass_geojson::FeatureDelta>,
) -> Result<Vec<PointChange>, FfiError> {
    let mut changes = Vec::new();
    for delta in &deltas {
        let SessionState {
            presentation,
            terrain,
            ..
        } = &mut *state;
        let change = presentation
            .apply_weather_delta_with_terrain_blocking(delta, |coordinate| {
                terrain_elevation_blocking(terrain, coordinate)
            })?;
        changes.extend(change);
    }
    Ok(changes)
}

fn terrain_elevation_blocking(
    terrain: &mut Option<TerrainArchive>,
    coordinate: Coordinate,
) -> Result<Option<f64>, FfiError> {
    let Some(terrain) = terrain else {
        return Ok(None);
    };
    terrain
        .elevation_m_blocking(coordinate.latitude_deg, coordinate.longitude_deg)
        .map_err(|source| FfiError::TerrainElevation {
            latitude_deg: coordinate.latitude_deg,
            longitude_deg: coordinate.longitude_deg,
            message: source.to_string(),
        })
}

fn weather_position_catalog(
    positions: Vec<WeatherStationPosition>,
) -> Result<BTreeMap<String, Wgs84Position>, FfiError> {
    let mut catalog = BTreeMap::new();
    for position in positions {
        let value =
            Wgs84Position::new(position.latitude_deg, position.longitude_deg).ok_or_else(|| {
                FfiError::WeatherStationPosition {
                    message: format!(
                        "station {} has WGS84 coordinates ({}, {}) outside the valid range",
                        position.station_id, position.latitude_deg, position.longitude_deg
                    ),
                }
            })?;
        catalog.insert(position.station_id, value);
    }
    Ok(catalog)
}

fn display_for_state(state: &SessionState, changes: Vec<PointChange>) -> DisplayBatch {
    let mut batch = state.presentation.adapt();
    batch.point_changes = changes;
    batch.into()
}

fn track_record_error(source: impl ToString) -> FfiError {
    FfiError::TrackRecord {
        message: source.to_string(),
    }
}

fn weather_record_error(source: impl ToString) -> FfiError {
    FfiError::WeatherRecord {
        message: source.to_string(),
    }
}
