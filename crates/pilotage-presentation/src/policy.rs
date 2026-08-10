//! Display policy and style catalogs.

use std::collections::BTreeMap;

use airmass_geojson::FeatureDelta as WeatherFeatureDelta;
use surveillance_core::{TrackDelta, TrackSnapshotHandle};

use crate::layer::{
    LayerPolicy, SourceObservation, TRAFFIC_LAYER_ID, WEATHER_ADVISORY_LAYER_ID,
    WEATHER_REPORT_LAYER_ID,
};
use crate::{DisplayBatch, PointChange, PointFeature, PointStyle, ShapeFeature, ShapeStyle};

pub(crate) const TRAFFIC_ACTIVE_STYLE: &str = "traffic-active";
pub(crate) const TRAFFIC_COASTING_STYLE: &str = "traffic-coasting";
pub(crate) const TRAFFIC_EMERGENCY_STYLE: &str = "traffic-emergency";
pub(crate) const TRAFFIC_ALTITUDE_STYLE: &str = "traffic-altitude";
pub(crate) const WEATHER_VFR_STYLE: &str = "weather-vfr";
pub(crate) const WEATHER_MVFR_STYLE: &str = "weather-mvfr";
pub(crate) const WEATHER_IFR_STYLE: &str = "weather-ifr";
pub(crate) const WEATHER_LIFR_STYLE: &str = "weather-lifr";
pub(crate) const WEATHER_UNKNOWN_STYLE: &str = "weather-unknown";
pub(crate) const ADVISORY_SIGMET_STYLE: &str = "advisory-sigmet";
pub(crate) const ADVISORY_CONVECTIVE_STYLE: &str = "advisory-convective";
pub(crate) const ADVISORY_AIRMET_STYLE: &str = "advisory-airmet";
pub(crate) const ADVISORY_G_AIRMET_STYLE: &str = "advisory-g-airmet";
pub(crate) const ADVISORY_CWA_STYLE: &str = "advisory-cwa";

/// Converts typed domain feature changes to display values.
#[derive(Clone, Debug, Default)]
pub struct PresentationAdapter {
    layers: LayerPolicy,
    traffic_points: BTreeMap<(u64, u64), PointFeature>,
    traffic_pads: BTreeMap<(u64, u64), ShapeFeature>,
    traffic_revisions: BTreeMap<(u64, u64), u64>,
    traffic_tracks: BTreeMap<(u64, u64), TrackSnapshotHandle>,
    weather_points: BTreeMap<String, PointFeature>,
    weather_shapes: BTreeMap<String, ShapeFeature>,
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
        self.traffic_revisions.insert(key, snapshot_revision);
        self.retain_track(key, delta);
        let source_change = surveillance_geojson::map_track_delta(delta)?;
        let change = crate::traffic::point_change(
            &source_change,
            producer_instance_id,
            snapshot_revision,
            self.traffic_points.get(&key),
        )?;
        self.apply_point_change(key, &change);
        self.layers.is_enabled(TRAFFIC_LAYER_ID).then_some(change)
    }

    /// Apply one ordered Airmass feature change.
    pub fn apply_weather_delta(&mut self, delta: &WeatherFeatureDelta) -> Option<PointChange> {
        let id = crate::weather::feature_id_for_delta(delta)?;
        self.apply_weather_shape(&id, delta);
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
        self.weather_points.clear();
        self.weather_shapes.clear();
    }

    /// Convert current traffic and weather values into one batch.
    #[must_use]
    pub fn adapt(&self) -> DisplayBatch {
        let mut batch = self.empty_batch();
        if self.layers.is_enabled(TRAFFIC_LAYER_ID) {
            batch.points.extend(self.traffic_points.values().cloned());
            batch.shapes.extend(self.traffic_pads.values().cloned());
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

    fn apply_weather_shape(&mut self, id: &str, delta: &WeatherFeatureDelta) {
        match delta {
            WeatherFeatureDelta::Upsert(_) => {
                if let Some(shape) = crate::weather::shape_change(delta) {
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
                    self.traffic_tracks.remove(&key);
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

    fn apply_point_change(&mut self, key: (u64, u64), change: &PointChange) {
        match change {
            PointChange::Upsert { point } => {
                match crate::traffic::altitude_pad(point) {
                    Some(pad) => self.traffic_pads.insert(key, pad),
                    // A track that loses its altitude must lose its pad with it, or the
                    // display keeps a height the track no longer reports.
                    None => self.traffic_pads.remove(&key),
                };
                self.traffic_points.insert(key, point.clone());
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

fn point_styles() -> Vec<PointStyle> {
    vec![
        traffic_point(TRAFFIC_ACTIVE_STYLE, [0, 229, 255, 255], 14.0, 40),
        traffic_point(TRAFFIC_COASTING_STYLE, [255, 179, 0, 255], 14.0, 30),
        traffic_point(TRAFFIC_EMERGENCY_STYLE, [255, 45, 45, 255], 18.0, 60),
        weather_point(WEATHER_VFR_STYLE, [0, 166, 81, 255], 8.0, 20),
        weather_point(WEATHER_MVFR_STYLE, [0, 102, 255, 255], 8.0, 20),
        weather_point(WEATHER_IFR_STYLE, [229, 57, 53, 255], 8.0, 20),
        weather_point(WEATHER_LIFR_STYLE, [176, 0, 181, 255], 8.0, 20),
        weather_point(WEATHER_UNKNOWN_STYLE, [117, 117, 117, 255], 8.0, 10),
    ]
}

fn traffic_point(id: &str, fill: [u8; 4], marker_size_points: f64, order: i32) -> PointStyle {
    point(id, fill, 0.0, Some("▲"), marker_size_points, order)
}

fn weather_point(id: &str, fill: [u8; 4], radius_points: f64, order: i32) -> PointStyle {
    point(id, fill, radius_points, None, 0.0, order)
}

fn point(
    id: &str,
    fill: [u8; 4],
    radius_points: f64,
    marker_text: Option<&str>,
    marker_size_points: f64,
    order: i32,
) -> PointStyle {
    PointStyle {
        id: id.into(),
        fill: crate::Color::rgba(fill[0], fill[1], fill[2], fill[3]),
        outline: crate::Color::rgba(255, 255, 255, 230),
        outline_width_points: 1.5,
        radius_points,
        marker_text: marker_text.map(str::to_owned),
        marker_size_points,
        marker_font_names: font_names(),
        marker_allows_overlap: true,
        label_color: crate::Color::rgba(255, 255, 255, 255),
        label_size_points: 12.0,
        label_font_names: font_names(),
        label_offset_x: 0.0,
        label_offset_y: 1.4,
        label_allows_overlap: false,
        order,
    }
}

fn shape_styles() -> Vec<ShapeStyle> {
    vec![
        extruded_shape(
            TRAFFIC_ALTITUDE_STYLE,
            [0, 229, 255, 150],
            [0, 229, 255, 255],
            60,
        ),
        extruded_shape(
            ADVISORY_AIRMET_STYLE,
            [255, 193, 7, 70],
            [255, 193, 7, 255],
            10,
        ),
        extruded_shape(
            ADVISORY_G_AIRMET_STYLE,
            [255, 152, 0, 70],
            [255, 152, 0, 255],
            20,
        ),
        extruded_shape(
            ADVISORY_CWA_STYLE,
            [156, 39, 176, 70],
            [206, 147, 216, 255],
            30,
        ),
        extruded_shape(
            ADVISORY_SIGMET_STYLE,
            [244, 67, 54, 75],
            [244, 67, 54, 255],
            40,
        ),
        extruded_shape(
            ADVISORY_CONVECTIVE_STYLE,
            [213, 0, 0, 85],
            [255, 82, 82, 255],
            50,
        ),
    ]
}

fn extruded_shape(id: &str, fill: [u8; 4], outline: [u8; 4], order: i32) -> ShapeStyle {
    ShapeStyle {
        extruded: true,
        ..shape(id, fill, outline, order)
    }
}

fn shape(id: &str, fill: [u8; 4], outline: [u8; 4], order: i32) -> ShapeStyle {
    ShapeStyle {
        id: id.into(),
        fill: crate::Color::rgba(fill[0], fill[1], fill[2], fill[3]),
        outline: crate::Color::rgba(outline[0], outline[1], outline[2], outline[3]),
        outline_width_points: 2.0,
        label_color: crate::Color::rgba(255, 255, 255, 255),
        label_size_points: 12.0,
        label_font_names: font_names(),
        label_offset_x: 0.0,
        label_offset_y: 0.0,
        label_allows_overlap: false,
        extruded: false,
        order,
    }
}

fn font_names() -> Vec<String> {
    vec![
        "Open Sans Regular".into(),
        "Arial Unicode MS Regular".into(),
    ]
}
