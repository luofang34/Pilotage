//! Errors from display-value conversion.

use thiserror::Error;

/// A snapshot value cannot become a display value.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PresentationError {
    /// A supported weather payload is not valid JSON.
    #[error("weather product {product_id} with media type {media_type} is not valid JSON")]
    WeatherPayloadJson {
        /// Product that has the invalid payload.
        product_id: String,
        /// Declared media type.
        media_type: String,
        /// JSON decode error.
        #[source]
        source: serde_json::Error,
    },
    /// A coordinate is outside its valid range.
    #[error(
        "weather product {product_id} has invalid coordinate ({latitude_deg}, {longitude_deg})"
    )]
    InvalidCoordinate {
        /// Product that has the invalid coordinate.
        product_id: String,
        /// Latitude in degrees.
        latitude_deg: f64,
        /// Longitude in degrees.
        longitude_deg: f64,
    },
    /// An advisory polygon has no valid exterior ring.
    #[error("weather product {product_id} has no closed exterior ring")]
    InvalidAdvisoryShape {
        /// Product that has the invalid polygon.
        product_id: String,
    },
}
