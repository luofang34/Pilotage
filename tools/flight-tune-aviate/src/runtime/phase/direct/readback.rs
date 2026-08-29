//! Validating what one direct command published before it is scored.
//!
//! The transport states that a command reached the flight controller. It
//! does not state that the record which describes that command is fit to
//! publish as evidence. This module is that second check: the record has
//! to agree with the durable prepared intent it closes, name the run
//! intent the campaign asked for, carry the causal order its own times
//! claim, and report an effective setpoint inside the declared tolerance.
//!
//! A record that fails any of these is quarantined, never scored. Evidence
//! that describes a command nobody can re-derive is worse than no
//! evidence, because a reader cannot tell which it is.

use flight_tune::Digest;

use crate::direct_transport::{DirectCommandRecord, DirectSetpoint};
use crate::runtime::AviateRuntimeError;

use super::ledger::DirectIntentRecord;

/// What one published direct record must agree with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PublicationContext {
    /// The exact run intent the campaign asked for.
    pub run_intent_digest: Digest,
    /// The direct transport that holds authority for this run.
    pub transport_identity_digest: Digest,
    /// The declared numeric tolerance for a target comparison.
    pub tolerance: f64,
}

/// Validates one direct command record before it becomes evidence.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the record disagrees with its
/// prepared intent, names another run intent or transport, states times
/// that are not causally ordered, or reports a target outside tolerance.
pub fn validate_publication(
    record: &DirectCommandRecord,
    intent: &DirectIntentRecord,
    context: &PublicationContext,
) -> Result<(), AviateRuntimeError> {
    require_intent_agreement(record, intent)?;
    if record.run_intent_digest != context.run_intent_digest
        || record.transport_identity_digest != context.transport_identity_digest
    {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "the record names another run intent or transport",
        });
    }
    require_causal_times(record)?;
    require_finite_setpoints(record)?;
    if !record
        .transmitted
        .matches_within(&record.requested, context.tolerance)
    {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "the transmitted setpoint left the requested target",
        });
    }
    if !record
        .effective
        .matches_within(&record.transmitted, context.tolerance)
    {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "the effective setpoint left the transmitted target",
        });
    }
    Ok(())
}

fn require_intent_agreement(
    record: &DirectCommandRecord,
    intent: &DirectIntentRecord,
) -> Result<(), AviateRuntimeError> {
    if record.schema_version == 0 || intent.schema_version == 0 {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "a direct document has no schema version",
        });
    }
    if record.purpose != intent.purpose
        || record.envelope_digest != intent.envelope_digest
        || record.requested != intent.requested
        || record.run_intent_digest != intent.run_intent_digest
        || record.transport_identity_digest != intent.transport_identity_digest
    {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "the record does not close its durable prepared intent",
        });
    }
    Ok(())
}

/// Rejects a record whose own times cannot have happened in that order.
///
/// A request precedes its transmit, and a transmit precedes the sample
/// that reports its effect. A record that says otherwise describes an
/// effect measured before its cause.
fn require_causal_times(record: &DirectCommandRecord) -> Result<(), AviateRuntimeError> {
    let times = record.times;
    if times.requested_at_ns > times.transmitted_at_ns
        || times.transmitted_at_ns > times.effective_at_ns
    {
        return Err(AviateRuntimeError::DirectPublicationRejected {
            detail: "the record times are not causally ordered",
        });
    }
    Ok(())
}

fn require_finite_setpoints(record: &DirectCommandRecord) -> Result<(), AviateRuntimeError> {
    let setpoints: [&DirectSetpoint; 4] = [
        &record.baseline,
        &record.requested,
        &record.transmitted,
        &record.effective,
    ];
    if setpoints.iter().all(|setpoint| setpoint.is_finite()) && record.normalized.is_finite() {
        return Ok(());
    }
    Err(AviateRuntimeError::DirectPublicationRejected {
        detail: "the record carries a value that is not a number",
    })
}
