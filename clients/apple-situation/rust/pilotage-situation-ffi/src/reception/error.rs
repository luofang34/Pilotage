//! Typed failures in the radio-domain composition path.

use airmass_aero_link::AdapterError;
use airmass_core::StoreError;
use surveillance_aero_link::NormalizeError;
use surveillance_core::{IngestError, SnapshotPublicationError};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ReceptionError {
    #[error("reconnect generation {reconnect_generation} exceeds the source epoch range")]
    ReconnectGenerationRange { reconnect_generation: u64 },
    #[error("reception event {index} has {actual} bytes; the limit is {limit}")]
    LineTooLong {
        index: usize,
        actual: usize,
        limit: usize,
    },
    #[error("reception event {index} is not a serialized AeroLink ReceptionEvent")]
    Decode {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("Surveillance could not serialize a track record at {monotonic_micros} microseconds")]
    TrackEncode {
        monotonic_micros: u64,
        #[source]
        source: serde_json::Error,
    },
    #[error("Surveillance rejected the reception at {received_at_micros} microseconds")]
    TrafficIngest {
        received_at_micros: u64,
        #[source]
        source: IngestError,
    },
    #[error("Surveillance cannot normalize the reception at {received_at_micros} microseconds")]
    TrafficRefusal {
        received_at_micros: u64,
        #[source]
        source: NormalizeError,
    },
    #[error("Surveillance returned an unsupported outcome at {received_at_micros} microseconds")]
    TrafficOutcome { received_at_micros: u64 },
    #[error("Surveillance could not advance to {monotonic_micros} microseconds")]
    TrafficAdvance {
        monotonic_micros: u64,
        #[source]
        source: SnapshotPublicationError,
    },
    #[error("Airmass state could not start for producer {producer_instance_id}")]
    WeatherConfiguration {
        producer_instance_id: u64,
        #[source]
        source: Box<StoreError>,
    },
    #[error("Airmass could not adapt the FIS-B frame at {received_at_micros} microseconds")]
    WeatherAdapter {
        received_at_micros: u64,
        #[source]
        source: AdapterError,
    },
    #[error("Airmass rejected product {product_id} at {monotonic_micros} microseconds")]
    WeatherStore {
        product_id: String,
        monotonic_micros: u64,
        #[source]
        source: Box<StoreError>,
    },
    #[error("Airmass could not advance to {monotonic_micros} microseconds")]
    WeatherAdvance {
        monotonic_micros: u64,
        #[source]
        source: Box<StoreError>,
    },
    #[error("Airmass could not serialize a weather record at {monotonic_micros} microseconds")]
    WeatherEncode {
        monotonic_micros: u64,
        #[source]
        source: serde_json::Error,
    },
}
