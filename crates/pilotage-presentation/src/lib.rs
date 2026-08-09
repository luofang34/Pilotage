//! Portable display policy for traffic and weather snapshots.
//!
//! The crate converts immutable domain values into display values. It does
//! not contain a platform API or an output encoding.

mod error;
mod model;
mod policy;
mod traffic;
mod weather;

#[cfg(test)]
mod tests;

pub use error::PresentationError;
pub use model::{
    Color, Coordinate, CoordinateRing, DisplayBatch, PointFeature, PointStyle, ShapeFeature,
    ShapeStyle,
};
pub use policy::PresentationAdapter;
pub use weather::{WEATHER_ADVISORY_MEDIA_TYPE, WEATHER_OBSERVATION_MEDIA_TYPE};
