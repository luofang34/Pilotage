//! Conversion of portable AeroLink receptions into typed domain records.

mod error;
mod traffic;
mod weather;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, MutexGuard};

use aero_link::ReceptionEvent;

pub(crate) use error::ReceptionError;
use traffic::TrafficPipeline;
use weather::WeatherPipeline;

use crate::{FfiError, RadioRecordBatch};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ReceptionTally {
    events_consumed: u64,
    traffic_observations: u64,
    traffic_refusals: u64,
    weather_products: u64,
}

#[derive(Debug, Default)]
struct DomainRecords {
    track: Vec<String>,
    weather: Vec<String>,
}

struct ReceptionPipeline {
    traffic: TrafficPipeline,
    weather: WeatherPipeline,
    last_monotonic_micros: u64,
}

impl ReceptionPipeline {
    fn new(producer_instance_id: u64) -> Result<Self, ReceptionError> {
        Ok(Self {
            traffic: TrafficPipeline::new(producer_instance_id)?,
            weather: WeatherPipeline::new(producer_instance_id)?,
            last_monotonic_micros: 0,
        })
    }

    fn accept(
        &mut self,
        line: String,
        reconnect_generation: u64,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<(ReceptionTally, DomainRecords), ReceptionError> {
        let event = decode_event(line, reconnect_generation)?;
        let operation_micros = self.operation_time(monotonic_micros, event.received_at_micros);
        let (traffic, mut track) = self.traffic.accept(&event)?;
        let (weather, mut weather_records) =
            self.weather.accept(&event, utc_millis, operation_micros)?;
        let advanced = self.advance(utc_millis, operation_micros)?;
        track.extend(advanced.track);
        weather_records.extend(advanced.weather);
        Ok((
            ReceptionTally {
                events_consumed: 1,
                traffic_observations: traffic.observations,
                traffic_refusals: traffic.refusals,
                weather_products: weather.products,
            },
            DomainRecords {
                track,
                weather: weather_records,
            },
        ))
    }

    fn advance(
        &mut self,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<DomainRecords, ReceptionError> {
        let operation_micros = self.operation_time(monotonic_micros, 0);
        Ok(DomainRecords {
            track: self.traffic.advance(operation_micros)?,
            weather: self.weather.advance(utc_millis, operation_micros)?,
        })
    }

    fn operation_time(&mut self, requested: u64, received: u64) -> u64 {
        let current = self.last_monotonic_micros.max(requested).max(received);
        self.last_monotonic_micros = current;
        current
    }
}

struct RadioState {
    producer_instance_id: u64,
    pipeline: ReceptionPipeline,
}

impl RadioState {
    fn new(producer_instance_id: u64) -> Result<Self, ReceptionError> {
        Ok(Self {
            producer_instance_id,
            pipeline: ReceptionPipeline::new(producer_instance_id)?,
        })
    }

    fn reset(&mut self) -> Result<(), ReceptionError> {
        self.producer_instance_id = self.producer_instance_id.wrapping_add(1);
        self.pipeline = ReceptionPipeline::new(self.producer_instance_id)?;
        Ok(())
    }
}

/// Converts AeroLink reception lines into versioned domain records.
#[derive(uniffi::Object)]
pub struct RadioDomainSession {
    state: Mutex<RadioState>,
}

#[uniffi::export]
impl RadioDomainSession {
    /// Create empty radio-domain state.
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, FfiError> {
        let state = RadioState::new(1).map_err(radio_state_error)?;
        Ok(Arc::new(Self {
            state: Mutex::new(state),
        }))
    }

    /// Convert one serialized AeroLink reception event.
    pub fn accept_reception_event(
        &self,
        event_json: String,
        reconnect_generation: u64,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<RadioRecordBatch, FfiError> {
        let (tally, records) = self
            .lock_state()?
            .pipeline
            .accept(
                event_json,
                reconnect_generation,
                utc_millis,
                monotonic_micros,
            )
            .map_err(radio_input_error)?;
        Ok(record_batch(tally, records))
    }

    /// Advance traffic and weather lifecycle time.
    pub fn advance_time(
        &self,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<RadioRecordBatch, FfiError> {
        let records = self
            .lock_state()?
            .pipeline
            .advance(utc_millis, monotonic_micros)
            .map_err(radio_input_error)?;
        Ok(record_batch(ReceptionTally::default(), records))
    }

    /// Clear state that came from radio reception.
    pub fn reset(&self) -> Result<(), FfiError> {
        self.lock_state()?.reset().map_err(radio_state_error)
    }
}

impl RadioDomainSession {
    fn lock_state(&self) -> Result<MutexGuard<'_, RadioState>, FfiError> {
        self.state.lock().map_err(|source| FfiError::SessionState {
            message: source.to_string(),
        })
    }
}

fn decode_event(line: String, reconnect_generation: u64) -> Result<ReceptionEvent, ReceptionError> {
    if line.len() > surveillance_aero_link::replay::MAX_RECORD_LINE_BYTES {
        return Err(ReceptionError::LineTooLong {
            index: 0,
            actual: line.len(),
            limit: surveillance_aero_link::replay::MAX_RECORD_LINE_BYTES,
        });
    }
    let source_epoch = u32::try_from(reconnect_generation).map_err(|_| {
        ReceptionError::ReconnectGenerationRange {
            reconnect_generation,
        }
    })?;
    let mut event = serde_json::from_str::<ReceptionEvent>(&line)
        .map_err(|source| ReceptionError::Decode { index: 0, source })?;
    event.source = event.source.with_epoch(source_epoch);
    Ok(event)
}

fn record_batch(tally: ReceptionTally, records: DomainRecords) -> RadioRecordBatch {
    RadioRecordBatch {
        track_records: records.track,
        weather_records: records.weather,
        events_consumed: tally.events_consumed,
        traffic_observations: tally.traffic_observations,
        traffic_refusals: tally.traffic_refusals,
        weather_products: tally.weather_products,
    }
}

fn radio_state_error(source: ReceptionError) -> FfiError {
    FfiError::RadioState {
        message: source.to_string(),
    }
}

fn radio_input_error(source: ReceptionError) -> FfiError {
    FfiError::RadioInput {
        message: source.to_string(),
    }
}
