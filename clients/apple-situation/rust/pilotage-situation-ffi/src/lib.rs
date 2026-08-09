//! UniFFI facade for the Pilotage iOS situation client.
//!
//! This standalone crate links the Surveillance and Airmass domain crates.
//! Generated bindings and binary artifacts stay outside the source tree.

mod error;
mod reception;
mod records;
mod session;

pub use error::FfiError;
pub use reception::RadioDomainSession;
pub use records::{
    DisplayBatch, DisplayColor, DisplayCoordinate, DisplayCoordinateRing, DisplayPoint,
    DisplayPointChange, DisplayPointChangeKind, DisplayPointStyle, DisplayShape, DisplayShapeStyle,
    ProducerSchemaVersions, RadioRecordBatch, WeatherStationPosition,
};
pub use session::PresentationSession;

/// Get the facade version.
#[uniffi::export]
#[must_use]
pub fn ffi_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Get the linked producer schema versions.
#[uniffi::export]
#[must_use]
pub fn producer_schema_versions() -> ProducerSchemaVersions {
    ProducerSchemaVersions {
        aero_link: aero_link::CURRENT_RECEPTION_SCHEMA_VERSION,
        surveillance: surveillance_core::CURRENT_TRACK_SCHEMA_VERSION,
        airmass: airmass_core::CURRENT_WEATHER_SNAPSHOT_SCHEMA_VERSION,
    }
}

uniffi::setup_scaffolding!();
