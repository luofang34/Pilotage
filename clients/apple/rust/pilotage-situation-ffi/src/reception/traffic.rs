//! Live AeroLink traffic ingestion through Surveillance.

use aero_link::ReceptionEvent;
use surveillance_aero_link::replay::{ReceptionOutcome, ingest_event, refusal_ends_the_stream};
use surveillance_core::{
    EngineConfig, ProducerInstanceId, SurveillanceEngine, TrackDelta, TrackRecord,
};

use super::ReceptionError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TrafficTally {
    pub(super) observations: u64,
    pub(super) refusals: u64,
}

pub(super) struct TrafficPipeline {
    engine: SurveillanceEngine,
}

impl TrafficPipeline {
    pub(super) fn new(producer_instance_id: u64) -> Result<Self, ReceptionError> {
        let engine = SurveillanceEngine::new(
            EngineConfig::default(),
            ProducerInstanceId::new(producer_instance_id),
        )
        .map_err(|source| ReceptionError::TrafficConfiguration {
            producer_instance_id,
            source: Box::new(source),
        })?;
        Ok(Self { engine })
    }

    pub(super) fn accept(
        &mut self,
        event: &ReceptionEvent,
    ) -> Result<(TrafficTally, Vec<String>), ReceptionError> {
        let mut deltas = Vec::new();
        let outcome = ingest_event(event, &mut self.engine, |delta| deltas.push(delta.clone()))
            .map_err(|source| ReceptionError::TrafficIngest {
                received_at_micros: event.received_at_micros,
                source,
            })?;
        let tally = match outcome {
            // A field the engine could not read is not a refused observation: the rest
            // of the report was taken. Counting it as a refusal would say the reception
            // failed when it did not.
            ReceptionOutcome::Ingested { .. } => TrafficTally {
                observations: 1,
                refusals: 0,
            },
            ReceptionOutcome::NotTraffic => TrafficTally::default(),
            ReceptionOutcome::Refused(source) if refusal_ends_the_stream(&source) => {
                return Err(ReceptionError::TrafficRefusal {
                    received_at_micros: event.received_at_micros,
                    source,
                });
            }
            ReceptionOutcome::Refused(_) => TrafficTally {
                observations: 0,
                refusals: 1,
            },
            _ => {
                return Err(ReceptionError::TrafficOutcome {
                    received_at_micros: event.received_at_micros,
                });
            }
        };
        Ok((tally, encode_deltas(deltas, event.received_at_micros)?))
    }

    pub(super) fn advance(&mut self, monotonic_micros: u64) -> Result<Vec<String>, ReceptionError> {
        let mut deltas = Vec::new();
        self.engine
            .advance_time(monotonic_micros, |delta| deltas.push(delta.clone()))
            .map_err(|source| ReceptionError::TrafficAdvance {
                monotonic_micros,
                source,
            })?;
        encode_deltas(deltas, monotonic_micros)
    }
}

fn encode_deltas(
    deltas: Vec<TrackDelta>,
    monotonic_micros: u64,
) -> Result<Vec<String>, ReceptionError> {
    deltas
        .into_iter()
        .map(|delta| {
            serde_json::to_string(&TrackRecord::new(delta)).map_err(|source| {
                ReceptionError::TrackEncode {
                    monotonic_micros,
                    source,
                }
            })
        })
        .collect()
}
