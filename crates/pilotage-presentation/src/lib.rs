//! Portable display policy for typed traffic and weather features.
//!
//! The crate converts domain feature changes into display values. It does
//! not contain a platform API or an output encoding.

mod model;
mod policy;
mod traffic;
mod weather;

#[cfg(test)]
mod tests;

pub use model::{
    Color, Coordinate, CoordinateRing, DisplayBatch, PointChange, PointFeature, PointStyle,
    ShapeFeature, ShapeStyle,
};
pub use policy::PresentationAdapter;
