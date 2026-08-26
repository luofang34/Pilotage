//! Values that a display binding consumes.

use surveillance_core::HeadingReference;

/// An sRGB color with straight alpha.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
    /// Alpha channel.
    pub alpha: u8,
}

impl Color {
    /// Create a color from its channels.
    #[must_use]
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// A WGS84 coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinate {
    /// Latitude in degrees, positive north.
    pub latitude_deg: f64,
    /// Longitude in degrees, positive east.
    pub longitude_deg: f64,
}

impl Coordinate {
    /// Create a coordinate when both values are finite and in range.
    #[must_use]
    pub fn checked(latitude_deg: f64, longitude_deg: f64) -> Option<Self> {
        let valid = latitude_deg.is_finite()
            && longitude_deg.is_finite()
            && (-90.0..=90.0).contains(&latitude_deg)
            && (-180.0..=180.0).contains(&longitude_deg);
        valid.then_some(Self {
            latitude_deg,
            longitude_deg,
        })
    }
}

/// One closed polygon ring.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateRing {
    /// Coordinates in ring order.
    pub coordinates: Vec<Coordinate>,
}

/// Policy for one family of point features.
#[derive(Clone, Debug, PartialEq)]
pub struct PointStyle {
    /// Stable style identity.
    pub id: String,
    /// Point fill color.
    pub fill: Color,
    /// Point outline color.
    pub outline: Color,
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
    pub label_color: Color,
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
    /// Order among point styles. A greater value is above a smaller value.
    pub order: i32,
}

/// Policy for one family of polygon features.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeStyle {
    /// Stable style identity.
    pub id: String,
    /// Polygon fill color.
    pub fill: Color,
    /// Polygon outline color.
    pub outline: Color,
    /// Outline width in screen points.
    pub outline_width_points: f64,
    /// Text color.
    pub label_color: Color,
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
    /// Whether the renderer raises this shape to the heights its features carry.
    ///
    /// A style that is not extruded draws flat on the terrain surface however the feature
    /// states its heights, because a renderer without a terrain build cannot raise it.
    pub extruded: bool,
    /// Order among shape styles. A greater value is above a smaller value.
    pub order: i32,
}

/// One point ready for display.
#[derive(Clone, Debug, PartialEq)]
pub struct PointFeature {
    /// Stable feature identity.
    pub id: String,
    /// Stable application layer identity.
    pub layer_id: String,
    /// Feature position.
    pub coordinate: Coordinate,
    /// Style identity from the point style catalog.
    pub style_id: String,
    /// Primary label.
    pub label: Option<String>,
    /// Selected display altitude in feet.
    pub altitude_ft: Option<i32>,
    /// Clockwise rotation from geographic north.
    pub rotation_deg: f64,
    /// Whether the position was advanced from the newest report rather than reported.
    ///
    /// A projection is a guess between two facts. A display that draws it identically to
    /// a report tells a reader that an aircraft is somewhere it has not said it is.
    pub position_is_extrapolated: bool,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// One renderer-neutral change to the point feature set.
#[derive(Clone, Debug, PartialEq)]
pub enum PointChange {
    /// Place or move one point.
    Upsert {
        /// Complete point value.
        point: PointFeature,
    },
    /// Mark an existing point as stale.
    Stale {
        /// Stable feature identity.
        id: String,
        /// Style identity selected by display policy.
        style_id: String,
        /// Producer instance identity.
        producer_instance_id: u64,
        /// Snapshot revision.
        snapshot_revision: u64,
    },
    /// Remove one point.
    Remove {
        /// Stable feature identity.
        id: String,
        /// Feature that absorbs this point after a merge.
        transfer_to: Option<String>,
        /// Producer instance identity.
        producer_instance_id: u64,
        /// Snapshot revision.
        snapshot_revision: u64,
    },
}

/// One polygon ready for display.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeFeature {
    /// Stable feature identity.
    pub id: String,
    /// Stable application layer identity.
    pub layer_id: String,
    /// Polygon rings. The first ring is the exterior ring.
    pub rings: Vec<CoordinateRing>,
    /// Style identity from the shape style catalog.
    pub style_id: String,
    /// Feature label.
    pub label: Option<String>,
    /// Floor of the shape, in metres above the terrain surface beneath it.
    ///
    /// This value keeps the reported altitude when
    /// [`uses_reported_altitude_fallback`](Self::uses_reported_altitude_fallback) is true.
    pub base_above_terrain_m: Option<f64>,
    /// Ceiling of the shape, in metres above the terrain surface beneath it.
    pub top_above_terrain_m: Option<f64>,
    /// Whether the height keeps a reported altitude because terrain was not available.
    pub uses_reported_altitude_fallback: bool,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// Where the aircraft carrying this display is.
///
/// The receiver hears the aircraft's own transmission, and the surveillance domain marks
/// that return so it is not drawn as traffic beside itself. The position in it is still a
/// position, and it is the one a display should centre on: it is where the aircraft is,
/// which a tablet on the back seat is not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnshipFeature {
    /// Reported position.
    pub coordinate: Coordinate,
    /// Direction of travel over the ground, in degrees from true north, when reported.
    pub course_deg: Option<f64>,
    /// Speed over the ground in knots, when reported.
    ///
    /// A course without a speed says which way the aircraft is going and not how
    /// far it gets, so a display can draw the direction and not the vector.
    pub ground_speed_kt: Option<f64>,
    /// Direction the nose points, in degrees, when reported.
    ///
    /// Not the same as the course: in wind the two differ by the crab angle, and
    /// that difference is what tells a reader the aircraft is crabbing rather
    /// than turning.
    pub heading_deg: Option<f64>,
    /// Which north the heading is measured from.
    ///
    /// A heading is a number and a reference. Drawn against the wrong north it is
    /// wrong by the local variation, which is tens of degrees in places, so a
    /// heading whose reference is unstated is not drawn as a true heading.
    pub heading_reference: Option<HeadingReference>,
    /// Selected display altitude in feet.
    pub altitude_ft: Option<i32>,
    /// Producer instance identity.
    pub producer_instance_id: u64,
    /// Snapshot revision.
    pub snapshot_revision: u64,
}

/// One complete set of display values and its style catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayBatch {
    /// User-controlled display layers.
    pub layers: Vec<crate::layer::LayerControl>,
    /// Point style catalog.
    pub point_styles: Vec<PointStyle>,
    /// Shape style catalog.
    pub shape_styles: Vec<ShapeStyle>,
    /// Point features.
    pub points: Vec<PointFeature>,
    /// Point changes since the preceding batch.
    pub point_changes: Vec<PointChange>,
    /// Polygon features.
    pub shapes: Vec<ShapeFeature>,
    /// Traffic tracks that do not have a map position.
    pub positionless_traffic: Vec<crate::detail::TrafficListItem>,
    /// Detail values for retained traffic tracks.
    pub traffic_details: Vec<crate::detail::TrafficDetail>,
    /// Supported domain products that had no display value.
    pub omitted_products: u64,
    /// Where the aircraft is, when its own return has been heard.
    pub ownship: Option<OwnshipFeature>,
}

impl DisplayBatch {
    /// Add features and omission counts from another batch.
    pub fn append(&mut self, other: Self) {
        self.layers = other.layers;
        self.points.extend(other.points);
        self.point_changes.extend(other.point_changes);
        self.shapes.extend(other.shapes);
        self.positionless_traffic = other.positionless_traffic;
        self.traffic_details = other.traffic_details;
        self.omitted_products = self.omitted_products.wrapping_add(other.omitted_products);
    }
}
