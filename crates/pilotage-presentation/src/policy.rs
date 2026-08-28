//! Display policy and style catalogs.

use std::collections::BTreeMap;

use airmass_geojson::FeatureDelta as WeatherFeatureDelta;
use surveillance_core::{TrackDelta, TrackSnapshotHandle};

use crate::layer::{
    LayerPolicy, SourceObservation, TRAFFIC_LAYER_ID, WEATHER_ADVISORY_LAYER_ID,
    WEATHER_REPORT_LAYER_ID,
};
use crate::style::{point_styles, shape_styles};
use crate::{Coordinate, DisplayBatch, OwnshipFeature, PointChange, PointFeature, ShapeFeature};

/// Converts typed domain feature changes to display values.
#[derive(Clone, Debug, Default)]
pub struct PresentationAdapter {
    layers: LayerPolicy,
    traffic_points: BTreeMap<(u64, u64), PointFeature>,
    traffic_pads: BTreeMap<(u64, u64), ShapeFeature>,
    traffic_revisions: BTreeMap<(u64, u64), u64>,
    traffic_tracks: BTreeMap<(u64, u64), TrackSnapshotHandle>,
    /// Ground height under each track, kept so a pad can be rebuilt where the track is
    /// projected to be rather than only where it last reported.
    traffic_terrain_m: BTreeMap<(u64, u64), Option<f64>>,
    weather_points: BTreeMap<String, PointFeature>,
    weather_shapes: BTreeMap<String, ShapeFeature>,
    ownship: Option<OwnshipFeature>,
    now_micros: u64,
}

impl PresentationAdapter {
    /// Create an adapter with the Pilotage display policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty batch that contains the complete style catalog.
    #[must_use]
    pub fn empty_batch(&self) -> DisplayBatch {
        DisplayBatch {
            layers: self.layers.controls(
                !self.traffic_tracks.is_empty(),
                !self.weather_points.is_empty(),
                !self.weather_shapes.is_empty(),
            ),
            point_styles: point_styles(),
            shape_styles: shape_styles(),
            points: Vec::new(),
            point_changes: Vec::new(),
            shapes: Vec::new(),
            positionless_traffic: Vec::new(),
            traffic_details: Vec::new(),
            omitted_products: 0,
            ownship: self.ownship,
        }
    }

    /// Advance the display clock without changing domain state.
    pub fn advance_time(&mut self, now_micros: u64) {
        self.now_micros = self.now_micros.max(now_micros);
    }

    /// Record raw source facts from the composition host.
    pub fn observe_sources(&mut self, sources: SourceObservation) {
        self.layers.observe_sources(sources);
    }

    /// Set the visibility of one known layer.
    pub fn set_layer_enabled(&mut self, id: &str, enabled: bool) -> bool {
        self.layers.set_enabled(id, enabled)
    }

    /// Apply one ordered Surveillance delta.
    pub fn apply_traffic_delta(&mut self, delta: &TrackDelta) -> Option<PointChange> {
        let (key, snapshot_revision, change) = self.prepare_traffic_delta(delta)?;
        self.commit_traffic_delta(key, snapshot_revision, delta, change.as_ref(), None);
        change.filter(|_| self.layers.is_enabled(TRAFFIC_LAYER_ID))
    }

    /// Apply one ordered Surveillance delta and read terrain for a positioned upsert.
    ///
    /// # Errors
    ///
    /// Returns the terrain reader error without changing retained display state.
    pub fn apply_traffic_delta_with_terrain_blocking<E>(
        &mut self,
        delta: &TrackDelta,
        mut elevation_at: impl FnMut(Coordinate) -> Result<Option<f64>, E>,
    ) -> Result<Option<PointChange>, E> {
        let Some((key, snapshot_revision, change)) = self.prepare_traffic_delta(delta) else {
            return Ok(None);
        };
        let terrain_elevation_m = match change.as_ref() {
            Some(PointChange::Upsert { point }) if point.altitude_ft.is_some() => {
                elevation_at(point.coordinate)?
            }
            Some(PointChange::Stale { .. } | PointChange::Remove { .. }) | None => None,
            Some(PointChange::Upsert { .. }) => None,
        };
        self.commit_traffic_delta(
            key,
            snapshot_revision,
            delta,
            change.as_ref(),
            terrain_elevation_m,
        );
        Ok(change.filter(|_| self.layers.is_enabled(TRAFFIC_LAYER_ID)))
    }

    fn prepare_traffic_delta(
        &self,
        delta: &TrackDelta,
    ) -> Option<((u64, u64), u64, Option<PointChange>)> {
        let producer_instance_id = delta.producer_instance_id().get();
        let snapshot_revision = delta.snapshot_revision().get();
        let key = (producer_instance_id, delta.id().get());
        let is_newer = self
            .traffic_revisions
            .get(&key)
            .is_none_or(|current| snapshot_revision > *current);
        if !is_newer {
            return None;
        }
        let change = surveillance_geojson::map_track_delta(delta).and_then(|source_change| {
            crate::traffic::point_change(
                &source_change,
                producer_instance_id,
                snapshot_revision,
                self.traffic_points.get(&key),
            )
        });
        Some((key, snapshot_revision, change))
    }

    fn commit_traffic_delta(
        &mut self,
        key: (u64, u64),
        snapshot_revision: u64,
        delta: &TrackDelta,
        change: Option<&PointChange>,
        terrain_elevation_m: Option<f64>,
    ) {
        self.traffic_revisions.insert(key, snapshot_revision);
        self.retain_track(key, delta);
        if let Some(change) = change {
            self.apply_point_change(key, change, terrain_elevation_m);
        }
    }

    /// Apply one ordered Airmass feature change.
    pub fn apply_weather_delta(&mut self, delta: &WeatherFeatureDelta) -> Option<PointChange> {
        self.apply_weather_delta_with_sample(delta, None)
    }

    /// Apply one Airmass feature change and read terrain for an advisory footprint.
    ///
    /// # Errors
    ///
    /// Returns the terrain reader error without changing retained display state.
    pub fn apply_weather_delta_with_terrain_blocking<E>(
        &mut self,
        delta: &WeatherFeatureDelta,
        mut elevation_at: impl FnMut(Coordinate) -> Result<Option<f64>, E>,
    ) -> Result<Option<PointChange>, E> {
        let terrain_elevation_m = match crate::weather::terrain_coordinate(delta) {
            Some(coordinate) => elevation_at(coordinate)?,
            None => None,
        };
        Ok(self.apply_weather_delta_with_sample(delta, terrain_elevation_m))
    }

    fn apply_weather_delta_with_sample(
        &mut self,
        delta: &WeatherFeatureDelta,
        terrain_elevation_m: Option<f64>,
    ) -> Option<PointChange> {
        let id = crate::weather::feature_id_for_delta(delta)?;
        self.apply_weather_shape(&id, delta, terrain_elevation_m);
        let change = crate::weather::point_change(delta, self.weather_points.get(&id))?;
        match &change {
            PointChange::Upsert { point } => {
                self.weather_points.insert(id, point.clone());
            }
            PointChange::Remove { .. } => {
                self.weather_points.remove(&id);
            }
            PointChange::Stale { .. } => {}
        }
        self.layers
            .is_enabled(WEATHER_REPORT_LAYER_ID)
            .then_some(change)
    }

    /// Remove all weather values without changing traffic state.
    pub fn clear_weather(&mut self) -> Vec<PointChange> {
        self.weather_shapes.clear();
        let points = std::mem::take(&mut self.weather_points);
        let changes: Vec<_> = points
            .into_iter()
            .map(|(id, point)| PointChange::Remove {
                id,
                transfer_to: None,
                producer_instance_id: point.producer_instance_id,
                snapshot_revision: point.snapshot_revision,
            })
            .collect();
        if self.layers.is_enabled(WEATHER_REPORT_LAYER_ID) {
            changes
        } else {
            Vec::new()
        }
    }

    /// Clear state supplied by radio reception and keep application controls.
    pub fn clear_radio_state(&mut self) {
        self.traffic_points.clear();
        self.traffic_pads.clear();
        self.traffic_revisions.clear();
        self.traffic_tracks.clear();
        self.traffic_terrain_m.clear();
        self.weather_points.clear();
        self.weather_shapes.clear();
        self.ownship = None;
    }

    /// Convert current traffic and weather values into one batch.
    #[must_use]
    pub fn adapt(&self) -> DisplayBatch {
        let mut batch = self.empty_batch();
        if self.layers.is_enabled(TRAFFIC_LAYER_ID) {
            // Drawn where the track is now, not where it last said it was. A map redraws
            // far more often than reports arrive, so a reported position alone makes a
            // target step from one report to the next.
            for (key, point) in &self.traffic_points {
                let drawn = self.projected_point(*key, point);
                if let Some(pad) = crate::traffic::altitude_pad(
                    &drawn,
                    self.traffic_terrain_m.get(key).copied().flatten(),
                ) {
                    batch.shapes.push(pad);
                }
                batch.points.push(drawn);
            }
            batch.positionless_traffic = self
                .traffic_tracks
                .values()
                .filter_map(|track| crate::detail::positionless_item(track, self.now_micros))
                .collect();
        }
        if self.layers.is_enabled(WEATHER_REPORT_LAYER_ID) {
            batch.points.extend(self.weather_points.values().cloned());
        }
        if self.layers.is_enabled(WEATHER_ADVISORY_LAYER_ID) {
            batch.shapes.extend(self.weather_shapes.values().cloned());
        }
        batch.traffic_details = self
            .traffic_tracks
            .values()
            .map(|track| crate::detail::detail_for(track, self.now_micros))
            .collect();
        batch
    }

    /// The point as it should be drawn now.
    ///
    /// Returns the point unchanged when the engine will not project: the guess is
    /// bounded there, and a display that projects further than the producer allows is
    /// inventing a position rather than showing one.
    fn projected_point(&self, key: (u64, u64), point: &PointFeature) -> PointFeature {
        let Some(track) = self.traffic_tracks.get(&key) else {
            return point.clone();
        };
        let Some(projection) =
            surveillance_core::project_position(track.snapshot(), self.now_micros)
        else {
            return point.clone();
        };
        if projection.basis != surveillance_core::ProjectionBasis::Extrapolated {
            return point.clone();
        }
        let Some(coordinate) = Coordinate::checked(
            projection.position.latitude_deg,
            projection.position.longitude_deg,
        ) else {
            return point.clone();
        };
        PointFeature {
            coordinate,
            altitude_ft: projection.pressure_altitude_ft.or(point.altitude_ft),
            position_is_extrapolated: true,
            ..point.clone()
        }
    }

    fn apply_weather_shape(
        &mut self,
        id: &str,
        delta: &WeatherFeatureDelta,
        terrain_elevation_m: Option<f64>,
    ) {
        match delta {
            WeatherFeatureDelta::Upsert(_) => {
                if let Some(shape) = crate::weather::shape_change(delta, terrain_elevation_m) {
                    self.weather_shapes.insert(id.to_owned(), shape);
                }
            }
            WeatherFeatureDelta::Remove { .. } => {
                self.weather_shapes.remove(id);
            }
            _ => {}
        }
    }

    fn retain_track(&mut self, key: (u64, u64), delta: &TrackDelta) {
        match delta {
            TrackDelta::Created(handle)
            | TrackDelta::Updated(handle)
            | TrackDelta::Coasting(handle) => {
                self.now_micros = self
                    .now_micros
                    .max(handle.snapshot().last_observed_at_micros);
                if handle.snapshot().ownship_shadow {
                    // The aircraft's own return is not traffic, and it is not nothing
                    // either: it carries where the aircraft is.
                    self.traffic_tracks.remove(&key);
                    self.ownship = ownship_feature(handle);
                } else {
                    self.traffic_tracks.insert(key, handle.clone());
                }
            }
            TrackDelta::Removed { .. } => {
                self.traffic_tracks.remove(&key);
            }
            _ => {}
        }
    }

    fn apply_point_change(
        &mut self,
        key: (u64, u64),
        change: &PointChange,
        terrain_elevation_m: Option<f64>,
    ) {
        match change {
            PointChange::Upsert { point } => {
                match crate::traffic::altitude_pad(point, terrain_elevation_m) {
                    Some(pad) => self.traffic_pads.insert(key, pad),
                    // A track that loses its altitude must lose its pad with it, or the
                    // display keeps a height the track no longer reports.
                    None => self.traffic_pads.remove(&key),
                };
                self.traffic_points.insert(key, point.clone());
                self.traffic_terrain_m.insert(key, terrain_elevation_m);
            }
            PointChange::Stale {
                style_id,
                snapshot_revision,
                ..
            } => {
                if let Some(point) = self.traffic_points.get_mut(&key) {
                    point.style_id.clone_from(style_id);
                    point.snapshot_revision = *snapshot_revision;
                }
            }
            PointChange::Remove { .. } => {
                self.traffic_points.remove(&key);
                self.traffic_pads.remove(&key);
            }
        }
    }
}

/// Read where the aircraft is from its own return.
///
/// A return with no position says the aircraft was heard and not where it was, so it
/// yields nothing rather than a coordinate the track never carried.
fn ownship_feature(handle: &surveillance_core::TrackSnapshotHandle) -> Option<OwnshipFeature> {
    let snapshot = handle.snapshot();
    let position = snapshot.position.as_ref()?;
    let coordinate =
        Coordinate::checked(position.value.latitude_deg, position.value.longitude_deg)?;
    let velocity = snapshot.velocity.as_ref().map(|field| &field.value);
    Some(OwnshipFeature {
        coordinate,
        course_deg: velocity.and_then(|value| value.track_angle_deg_true),
        ground_speed_kt: velocity.and_then(|value| value.ground_speed_kt),
        heading_deg: velocity.and_then(|value| value.heading_deg),
        heading_reference: velocity.and_then(|value| value.heading_reference),
        altitude_ft: snapshot
            .pressure_altitude_ft
            .as_ref()
            .map(|field| field.value)
            .or_else(|| snapshot.geometric_altitude_ft.as_ref().map(|f| f.value)),
        producer_instance_id: handle.producer_instance_id().get(),
        snapshot_revision: handle.snapshot_revision().get(),
    })
}
