//! UniFFI facade for the Pilotage iOS situation client.
//!
//! This standalone crate links the Surveillance and Airmass domain crates.
//! Generated bindings and binary artifacts stay outside the source tree.

// UniFFI derive output uses a dynamic error.
#[allow(clippy::disallowed_types)]
mod error;
// UniFFI derive output uses a dynamic error.
#[allow(clippy::disallowed_types)]
mod link;
mod reception;
// UniFFI derive output uses a dynamic error.
#[allow(clippy::disallowed_types)]
mod records;
mod session;

pub use error::FfiError;
pub use link::{
    LinkCatalog, LinkConfig, LinkControlFeelIdentity, LinkControlFeelMode, LinkEvent,
    LinkIntentCapability, LinkObserver, LinkScope, LinkSession, LinkVehicle,
};
// Links the instrument bridge's scaffolding into this library, so one
// static library carries both namespaces and one bindgen run over it
// generates both Swift surfaces (ADR-0032's single-app composition).
pub use pilotage_instrument_apple_bridge as instrument_bridge;
pub use reception::RadioDomainSession;
pub use records::{
    DisplayBatch, DisplayColor, DisplayCoordinate, DisplayCoordinateRing, DisplayLayerControl,
    DisplayLayerSourceState, DisplayPoint, DisplayPointChange, DisplayPointChangeKind,
    DisplayPointStyle, DisplayShape, DisplayShapeStyle, DisplayTrafficDetail,
    DisplayTrafficDetailField, DisplayTrafficListItem, PresentationRadioBand,
    PresentationRadioState, PresentationReceiverObservation, PresentationSourceObservation,
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
