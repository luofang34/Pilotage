//! Live AeroLink FIS-B ingestion through Airmass.

use aero_link::ReceptionEvent;
use airmass_aero_link::AeroLinkAdapter;
use airmass_core::{
    EvaluationTime, MonotonicTime, ProducerInstanceId, StoreConfig, UtcTime, WeatherSnapshotHandle,
    WeatherSnapshotRecord, WeatherStore,
};

use super::ReceptionError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WeatherTally {
    pub(super) products: u64,
}

pub(super) struct WeatherPipeline {
    adapter: AeroLinkAdapter,
    store: WeatherStore,
}

impl WeatherPipeline {
    pub(super) fn new(producer_instance_id: u64) -> Result<Self, ReceptionError> {
        let store = WeatherStore::new(
            StoreConfig::default(),
            ProducerInstanceId::new(producer_instance_id),
        )
        .map_err(|source| ReceptionError::WeatherConfiguration {
            producer_instance_id,
            source: Box::new(source),
        })?;
        Ok(Self {
            adapter: AeroLinkAdapter::default(),
            store,
        })
    }

    pub(super) fn accept(
        &mut self,
        event: &ReceptionEvent,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<(WeatherTally, Vec<String>), ReceptionError> {
        let mut tally = WeatherTally::default();
        let mut records = Vec::new();
        let now = evaluation_time(utc_millis, monotonic_micros);
        for projection in event.information_frames() {
            let batch = self
                .adapter
                .accept_projection(projection)
                .map_err(|source| ReceptionError::WeatherAdapter {
                    received_at_micros: event.received_at_micros,
                    source,
                })?;
            for ingress in batch.into_values() {
                tally.products = tally.products.wrapping_add(1);
                if let Some(snapshot) = self.accept_ingress(ingress, now, monotonic_micros)? {
                    records.push(encode_snapshot(snapshot, monotonic_micros)?);
                }
            }
        }
        Ok((tally, records))
    }

    pub(super) fn advance(
        &mut self,
        utc_millis: i64,
        monotonic_micros: u64,
    ) -> Result<Vec<String>, ReceptionError> {
        let publication = self
            .store
            .advance_time(evaluation_time(utc_millis, monotonic_micros))
            .map_err(|source| ReceptionError::WeatherAdvance {
                monotonic_micros,
                source: Box::new(source),
            })?
            .into_publication();
        publication.map_or_else(
            || Ok(Vec::new()),
            |snapshot| Ok(vec![encode_snapshot(snapshot, monotonic_micros)?]),
        )
    }

    fn accept_ingress(
        &mut self,
        ingress: airmass_core::WeatherIngress,
        now: EvaluationTime,
        monotonic_micros: u64,
    ) -> Result<Option<WeatherSnapshotHandle>, ReceptionError> {
        let product_id = ingress.product_id().as_str().to_owned();
        let publication = self
            .store
            .accept(ingress, now)
            .map_err(|source| ReceptionError::WeatherStore {
                product_id,
                monotonic_micros,
                source: Box::new(source),
            })?
            .into_publication();
        Ok(publication)
    }
}

fn encode_snapshot(
    snapshot: WeatherSnapshotHandle,
    monotonic_micros: u64,
) -> Result<String, ReceptionError> {
    serde_json::to_string(&WeatherSnapshotRecord::new(snapshot.into_envelope())).map_err(|source| {
        ReceptionError::WeatherEncode {
            monotonic_micros,
            source,
        }
    })
}

const fn evaluation_time(utc_millis: i64, monotonic_micros: u64) -> EvaluationTime {
    EvaluationTime::new(
        UtcTime::from_unix_millis(utc_millis),
        MonotonicTime::from_micros(monotonic_micros),
    )
}
