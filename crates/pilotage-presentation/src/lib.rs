//! Portable display policy for typed traffic and weather features.
//!
//! The crate converts domain feature changes into display values. It does
//! not contain a platform API or an output encoding.

mod detail;
mod layer;
mod model;
mod policy;
mod traffic;
mod weather;

#[cfg(test)]
mod tests;

pub use detail::{TrafficDetail, TrafficDetailField, TrafficListItem};
pub use layer::{
    LayerControl, LayerSourceState, RadioBand, RadioReceiverObservation, RadioReceptionState,
    SourceObservation, TERRAIN_LAYER_ID, TRAFFIC_LAYER_ID, WEATHER_ADVISORY_LAYER_ID,
    WEATHER_REPORT_LAYER_ID,
};
pub use model::{
    Color, Coordinate, CoordinateRing, DisplayBatch, PointChange, PointFeature, PointStyle,
    ShapeFeature, ShapeStyle,
};
pub use policy::PresentationAdapter;
