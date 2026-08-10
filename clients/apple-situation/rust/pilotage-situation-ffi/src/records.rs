//! Flat records that cross the Apple FFI boundary.

mod presentation;

pub use presentation::{
    DisplayLayerControl, DisplayLayerSourceState, DisplayTrafficDetail, DisplayTrafficDetailField,
    DisplayTrafficListItem, PresentationRadioBand, PresentationRadioState,
    PresentationReceiverObservation, PresentationSourceObservation,
};

/// Schema versions of the linked domain producers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ProducerSchemaVersions {
    /// AeroLink reception event schema version.
    pub aero_link: u16,
    /// Surveillance track schema version.
    pub surveillance: u16,
    /// Airmass weather snapshot schema version.
    pub airmass: u16,
}

/// One weather station position supplied by the application.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct WeatherStationPosition {
    /// Published weather station identity.
    pub station_id: String,
    /// WGS84 latitude in degrees.
    pub latitude_deg: f64,
    /// WGS84 longitude in degrees.
    pub longitude_deg: f64,
}

/// One color in the sRGB color space.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DisplayColor {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
    /// Alpha channel.
    pub alpha: u8,
}

/// One WGS84 coordinate.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct DisplayCoordinate {
    /// Latitude in degrees, positive north.
    pub latitude_deg: f64,
    /// Longitude in degrees, positive east.
    pub longitude_deg: f64,
}

/// One polygon ring.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayCoordinateRing {
    /// Coordinates in ring order.
    pub coordinates: Vec<DisplayCoordinate>,
}

/// Style for one family of point features.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayPointStyle {
    /// Stable style identity.
    pub id: String,
    /// Point fill color.
    pub fill: DisplayColor,
    /// Point outline color.
    pub outline: DisplayColor,
    /// Point outline width in screen points.
    pub outline_width_points: f64,
    /// Point radius in screen points.
    pub radius_points: f64,
    /// Optional text mark for the point.
    pub marker_text: Option<String>,
    /// Text mark size in screen points.
    pub marker_size_points: f64,
    /// Font preference for the text mark.
    pub marker_font_names: Vec<String>,
    /// Whether the text mark can overlap another symbol.
    pub marker_allows_overlap: bool,
    /// Text color.
    pub label_color: DisplayColor,
    /// Text size in screen points.
    pub label_size_points: f64,
    /// Font preference for the label.
    pub label_font_names: Vec<String>,
    /// Horizontal text offset in text-em units.
    pub label_offset_x: f64,
    /// Vertical text offset in text-em units.
    pub label_offset_y: f64,
    /// Whether the label can overlap another symbol.
    pub label_allows_overlap: bool,
    /// Order among point styles.
    pub order: i32,
}

/// Style for one family of polygon features.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayShapeStyle {
    /// Stable style identity.
    pub id: String,
    /// Polygon fill color.
    pub fill: DisplayColor,
    /// Polygon outline color.
    pub outline: DisplayColor,
    /// Outline width in screen points.
    pub outline_width_points: f64,
    /// Text color.
    pub label_color: DisplayColor,
    /// Text size in screen points.
    pub label_size_points: f64,
    /// Font preference for the label.
    pub label_font_names: Vec<String>,
    /// Horizontal text offset in text-em units.
    pub label_offset_x: f64,
    /// Vertical text offset in text-em units.
    pub label_offset_y: f64,
    /// Whether the label can overlap another symbol.
    pub label_allows_overlap: bool,
    /// Order among shape styles.
    pub order: i32,
}

/// One point ready for the Swift display edge.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayPoint {
    /// Stable feature identity.
    pub id: String,
    /// Stable application layer identity.
    pub layer_id: String,
    /// Feature position.
    pub coordinate: DisplayCoordinate,
    /// Style identity.
    pub style_id: String,
    /// Primary label.
    pub label: Option<String>,
    /// Selected display altitude in feet.
    pub altitude_ft: Option<i32>,
    /// Clockwise rotation from geographic north.
    pub rotation_deg: f64,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// Operation for one point change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DisplayPointChangeKind {
    /// Place or move a point.
    Upsert,
    /// Mark a point as stale.
    Stale,
    /// Remove a point.
    Remove,
}

/// One point change at the Swift display edge.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayPointChange {
    /// Operation to apply.
    pub kind: DisplayPointChangeKind,
    /// Complete point for an upsert.
    pub point: Option<DisplayPoint>,
    /// Stable feature identity.
    pub id: String,
    /// Style selected for a stale point.
    pub style_id: Option<String>,
    /// Feature that absorbs this point after a merge.
    pub transfer_to: Option<String>,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// One polygon ready for the Swift display edge.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayShape {
    /// Stable feature identity.
    pub id: String,
    /// Stable application layer identity.
    pub layer_id: String,
    /// Polygon rings.
    pub rings: Vec<DisplayCoordinateRing>,
    /// Style identity.
    pub style_id: String,
    /// Feature label.
    pub label: Option<String>,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// One complete set of display values and styles.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayBatch {
    /// User-controlled display layers.
    pub layers: Vec<DisplayLayerControl>,
    /// Point style catalog.
    pub point_styles: Vec<DisplayPointStyle>,
    /// Polygon style catalog.
    pub shape_styles: Vec<DisplayShapeStyle>,
    /// Point features.
    pub points: Vec<DisplayPoint>,
    /// Point changes since the preceding batch.
    pub point_changes: Vec<DisplayPointChange>,
    /// Polygon features.
    pub shapes: Vec<DisplayShape>,
    /// Traffic tracks that do not have a map position.
    pub positionless_traffic: Vec<DisplayTrafficListItem>,
    /// Detail values for retained traffic tracks.
    pub traffic_details: Vec<DisplayTrafficDetail>,
    /// Products that had no display value.
    pub omitted_products: u64,
}

/// Versioned domain records emitted for one radio operation.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RadioRecordBatch {
    /// Versioned Surveillance track records.
    pub track_records: Vec<String>,
    /// Versioned Airmass weather snapshot records.
    pub weather_records: Vec<String>,
    /// AeroLink reception events consumed by this operation.
    pub events_consumed: u64,
    /// Traffic observations accepted by Surveillance.
    pub traffic_observations: u64,
    /// Traffic receptions refused without ending the stream.
    pub traffic_refusals: u64,
    /// Weather products accepted by Airmass.
    pub weather_products: u64,
}

impl From<pilotage_presentation::DisplayBatch> for DisplayBatch {
    fn from(value: pilotage_presentation::DisplayBatch) -> Self {
        Self {
            layers: value.layers.into_iter().map(Into::into).collect(),
            point_styles: value.point_styles.into_iter().map(Into::into).collect(),
            shape_styles: value.shape_styles.into_iter().map(Into::into).collect(),
            points: value.points.into_iter().map(Into::into).collect(),
            point_changes: value.point_changes.into_iter().map(Into::into).collect(),
            shapes: value.shapes.into_iter().map(Into::into).collect(),
            positionless_traffic: value
                .positionless_traffic
                .into_iter()
                .map(Into::into)
                .collect(),
            traffic_details: value.traffic_details.into_iter().map(Into::into).collect(),
            omitted_products: value.omitted_products,
        }
    }
}

impl From<pilotage_presentation::Color> for DisplayColor {
    fn from(value: pilotage_presentation::Color) -> Self {
        Self {
            red: value.red,
            green: value.green,
            blue: value.blue,
            alpha: value.alpha,
        }
    }
}

impl From<pilotage_presentation::Coordinate> for DisplayCoordinate {
    fn from(value: pilotage_presentation::Coordinate) -> Self {
        Self {
            latitude_deg: value.latitude_deg,
            longitude_deg: value.longitude_deg,
        }
    }
}

impl From<pilotage_presentation::PointStyle> for DisplayPointStyle {
    fn from(value: pilotage_presentation::PointStyle) -> Self {
        Self {
            id: value.id,
            fill: value.fill.into(),
            outline: value.outline.into(),
            outline_width_points: value.outline_width_points,
            radius_points: value.radius_points,
            marker_text: value.marker_text,
            marker_size_points: value.marker_size_points,
            marker_font_names: value.marker_font_names,
            marker_allows_overlap: value.marker_allows_overlap,
            label_color: value.label_color.into(),
            label_size_points: value.label_size_points,
            label_font_names: value.label_font_names,
            label_offset_x: value.label_offset_x,
            label_offset_y: value.label_offset_y,
            label_allows_overlap: value.label_allows_overlap,
            order: value.order,
        }
    }
}

impl From<pilotage_presentation::ShapeStyle> for DisplayShapeStyle {
    fn from(value: pilotage_presentation::ShapeStyle) -> Self {
        Self {
            id: value.id,
            fill: value.fill.into(),
            outline: value.outline.into(),
            outline_width_points: value.outline_width_points,
            label_color: value.label_color.into(),
            label_size_points: value.label_size_points,
            label_font_names: value.label_font_names,
            label_offset_x: value.label_offset_x,
            label_offset_y: value.label_offset_y,
            label_allows_overlap: value.label_allows_overlap,
            order: value.order,
        }
    }
}

impl From<pilotage_presentation::PointFeature> for DisplayPoint {
    fn from(value: pilotage_presentation::PointFeature) -> Self {
        Self {
            id: value.id,
            layer_id: value.layer_id,
            coordinate: value.coordinate.into(),
            style_id: value.style_id,
            label: value.label,
            altitude_ft: value.altitude_ft,
            rotation_deg: value.rotation_deg,
            producer_instance_id: value.producer_instance_id,
            snapshot_revision: value.snapshot_revision,
        }
    }
}

impl From<pilotage_presentation::PointChange> for DisplayPointChange {
    fn from(value: pilotage_presentation::PointChange) -> Self {
        match value {
            pilotage_presentation::PointChange::Upsert { point } => {
                let id = point.id.clone();
                let producer_instance_id = point.producer_instance_id;
                let snapshot_revision = point.snapshot_revision;
                Self {
                    kind: DisplayPointChangeKind::Upsert,
                    point: Some(point.into()),
                    id,
                    style_id: None,
                    transfer_to: None,
                    producer_instance_id,
                    snapshot_revision,
                }
            }
            pilotage_presentation::PointChange::Stale {
                id,
                style_id,
                producer_instance_id,
                snapshot_revision,
            } => Self {
                kind: DisplayPointChangeKind::Stale,
                point: None,
                id,
                style_id: Some(style_id),
                transfer_to: None,
                producer_instance_id,
                snapshot_revision,
            },
            pilotage_presentation::PointChange::Remove {
                id,
                transfer_to,
                producer_instance_id,
                snapshot_revision,
            } => Self {
                kind: DisplayPointChangeKind::Remove,
                point: None,
                id,
                style_id: None,
                transfer_to,
                producer_instance_id,
                snapshot_revision,
            },
        }
    }
}

impl From<pilotage_presentation::ShapeFeature> for DisplayShape {
    fn from(value: pilotage_presentation::ShapeFeature) -> Self {
        Self {
            id: value.id,
            layer_id: value.layer_id,
            rings: value
                .rings
                .into_iter()
                .map(|ring| DisplayCoordinateRing {
                    coordinates: ring.coordinates.into_iter().map(Into::into).collect(),
                })
                .collect(),
            style_id: value.style_id,
            label: value.label,
            producer_instance_id: value.producer_instance_id,
            snapshot_revision: value.snapshot_revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProducerSchemaVersions;

    #[test]
    fn linked_versions_are_nonzero() {
        let versions = crate::producer_schema_versions();
        assert_ne!(
            versions,
            ProducerSchemaVersions {
                aero_link: 0,
                surveillance: 0,
                airmass: 0,
            }
        );
        assert_ne!(versions.aero_link, 0);
        assert_ne!(versions.surveillance, 0);
        assert_ne!(versions.airmass, 0);
    }
}
