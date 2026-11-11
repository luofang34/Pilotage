//! Lossless mapping of adapter procedure progress to telemetry.

use pilotage_adapter_api::FlightProcedureSample;
use pilotage_protocol::wire;

pub(super) fn to_wire(sample: FlightProcedureSample) -> wire::FlightProcedureState {
    wire::FlightProcedureState {
        request: Some(wire::FlightProcedureRequest {
            session: Some(wire::SessionId {
                value: sample.request.session.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: sample.request.scope.as_str().to_owned(),
            }),
            generation: Some(wire::Generation {
                value: sample.request.generation.as_u64(),
            }),
            sequence: Some(wire::SequenceNum {
                value: sample.request.sequence.as_u32(),
            }),
            action_id: sample.request.action_id,
        }),
        adapter_incarnation: sample.adapter_incarnation.into_bytes().to_vec(),
        revision: sample.revision,
        changed_at: Some(wire::MonoTimestamp {
            nanos: sample.changed_at.as_nanos(),
        }),
        kind: sample.kind.to_wire() as i32,
        phase: sample.phase.to_wire() as i32,
        reason: sample
            .reason
            .map_or(wire::FlightProcedureReason::Unspecified, |reason| {
                reason.to_wire()
            }) as i32,
        fc_outcome: sample
            .fc_outcome
            .map_or(wire::CommandOutcome::Unspecified, |outcome| {
                outcome.to_wire()
            }) as i32,
    }
}

#[cfg(test)]
mod tests;
