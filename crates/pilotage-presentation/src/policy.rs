//! Display policy and style catalogs.

use airmass_core::WeatherSnapshotEnvelope;
use surveillance_core::TrackSnapshotHandle;

use crate::{DisplayBatch, PointStyle, PresentationError, ShapeStyle};

pub(crate) const TRAFFIC_ACTIVE_STYLE: &str = "traffic-active";
pub(crate) const TRAFFIC_COASTING_STYLE: &str = "traffic-coasting";
pub(crate) const TRAFFIC_EMERGENCY_STYLE: &str = "traffic-emergency";
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

/// Converts domain snapshots to display values.
#[derive(Clone, Copy, Debug, Default)]
pub struct PresentationAdapter;

impl PresentationAdapter {
    /// Create an adapter with the Pilotage display policy.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Create an empty batch that contains the complete style catalog.
    #[must_use]
    pub fn empty_batch(self) -> DisplayBatch {
        DisplayBatch {
            point_styles: point_styles(),
            shape_styles: shape_styles(),
            points: Vec::new(),
            shapes: Vec::new(),
            omitted_products: 0,
        }
    }

    /// Convert current traffic and weather snapshots into one batch.
    pub fn adapt(
        self,
        tracks: &[TrackSnapshotHandle],
        weather: Option<&WeatherSnapshotEnvelope>,
    ) -> Result<DisplayBatch, PresentationError> {
        let mut batch = self.empty_batch();
        batch
            .points
            .extend(tracks.iter().filter_map(crate::traffic::point_for_track));
        if let Some(snapshot) = weather {
            batch.append(crate::weather::features_for_weather(snapshot)?);
        }
        Ok(batch)
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
        shape(
            ADVISORY_AIRMET_STYLE,
            [255, 193, 7, 70],
            [255, 193, 7, 255],
            10,
        ),
        shape(
            ADVISORY_G_AIRMET_STYLE,
            [255, 152, 0, 70],
            [255, 152, 0, 255],
            20,
        ),
        shape(
            ADVISORY_CWA_STYLE,
            [156, 39, 176, 70],
            [206, 147, 216, 255],
            30,
        ),
        shape(
            ADVISORY_SIGMET_STYLE,
            [244, 67, 54, 75],
            [244, 67, 54, 255],
            40,
        ),
        shape(
            ADVISORY_CONVECTIVE_STYLE,
            [213, 0, 0, 85],
            [255, 82, 82, 255],
            50,
        ),
    ]
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
        order,
    }
}

fn font_names() -> Vec<String> {
    vec![
        "Open Sans Regular".into(),
        "Arial Unicode MS Regular".into(),
    ]
}
