//! Style catalog: what each mark and each shape looks like.
//!
//! Separate from the policy that decides which of them a batch holds, because a
//! colour changes for reasons that have nothing to do with what the display knows.

use crate::{PointStyle, ShapeStyle};

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

pub(crate) fn point_styles() -> Vec<PointStyle> {
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

pub(crate) fn traffic_point(
    id: &str,
    fill: [u8; 4],
    marker_size_points: f64,
    order: i32,
) -> PointStyle {
    point(id, fill, 0.0, Some("▲"), marker_size_points, order)
}

pub(crate) fn weather_point(id: &str, fill: [u8; 4], radius_points: f64, order: i32) -> PointStyle {
    point(id, fill, radius_points, None, 0.0, order)
}

pub(crate) fn point(
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

pub(crate) fn shape_styles() -> Vec<ShapeStyle> {
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

pub(crate) fn extruded_shape(id: &str, fill: [u8; 4], outline: [u8; 4], order: i32) -> ShapeStyle {
    ShapeStyle {
        extruded: true,
        ..shape(id, fill, outline, order)
    }
}

pub(crate) fn shape(id: &str, fill: [u8; 4], outline: [u8; 4], order: i32) -> ShapeStyle {
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

pub(crate) fn font_names() -> Vec<String> {
    vec![
        "Open Sans Regular".into(),
        "Arial Unicode MS Regular".into(),
    ]
}
