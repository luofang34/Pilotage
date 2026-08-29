//! Driving one exact direct step through the durable ledger.
//!
//! Every direct command this runtime sends follows the same order, and the
//! order is the point: make the prepared intent durable, enact it, make
//! the result durable, validate the record, and only then let it count as
//! evidence. A failure at any step leaves the ledger able to say what the
//! run had already asked the vehicle to do.

pub mod ledger;
pub mod readback;

use flight_tune::{ControlChannel, ControlFamily, StimulusEnvelope, StimulusMapping};

use crate::direct_transport::{
    DirectBaselineRequest, DirectCommandSender, DirectEnactment, DirectStepRequest,
};

use crate::runtime::AviateRuntimeError;
use crate::runtime::direct::DirectRunAuthority;
use crate::runtime::math::clamp_normalized;

use ledger::DirectIntentStore;
use readback::validate_publication;

/// What one direct command asked the vehicle for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectStepOutcome {
    /// The command reached the flight controller and became evidence.
    Enacted,
    /// The raw source has not reached the command time. Nothing was sent.
    Pending,
    /// The raw source carries no exact sample. Nothing was sent.
    NoExactSource,
}

/// Freezes the direct baseline this run measures every step from.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the transport refuses the request or
/// the baseline block ends without a stable matching readback.
pub fn freeze_baseline_blocking<S: DirectCommandSender + ?Sized>(
    authority: &mut DirectRunAuthority,
    sender: &mut S,
    request: &DirectBaselineRequest,
) -> Result<(), AviateRuntimeError> {
    authority
        .transport_mut()
        .freeze_baseline_blocking(sender, request)
        .map(|_frozen| ())
        .map_err(|source| AviateRuntimeError::DirectTransport { source })
}

/// Sends one exact direct step, bracketed by the durable ledger.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the stimulus is not the direct
/// family, when the ledger cannot make the intent or the result durable,
/// when the transport refuses the command, or when the resulting record
/// fails publication validation.
pub fn send_step_blocking<S, L>(
    authority: &mut DirectRunAuthority,
    sender: &mut S,
    store: &mut L,
    stimulus: &DirectStepRequest,
    release: bool,
) -> Result<DirectStepOutcome, AviateRuntimeError>
where
    S: DirectCommandSender + ?Sized,
    L: DirectIntentStore + ?Sized,
{
    let transport = authority.transport_mut();
    let prepared = if release {
        transport.prepare_release(stimulus)
    } else {
        transport.prepare_step(stimulus)
    }
    .map_err(|source| AviateRuntimeError::DirectTransport { source })?;

    // The intent is durable before enactment can put a datagram on the
    // link, so a stop between the two lines leaves a readable prepared
    // frame rather than an unrecorded command.
    let intent = authority.ledger_mut().prepare(store, &prepared)?;
    let enactment = authority
        .transport_mut()
        .enact_blocking(sender, &prepared)
        .map_err(|source| AviateRuntimeError::DirectTransport { source })?;
    let result = authority.ledger_mut().resolve(store, &intent, &enactment)?;
    let _sequence = result.sequence;

    match enactment {
        DirectEnactment::Enacted(record) => {
            validate_publication(&record, &intent, authority.publication_context())?;
            authority.record(*record);
            Ok(DirectStepOutcome::Enacted)
        }
        DirectEnactment::Pending => Ok(DirectStepOutcome::Pending),
        DirectEnactment::NoExactSource => Ok(DirectStepOutcome::NoExactSource),
    }
}

/// Builds one direct step request from a mission stimulus.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the family is not the direct
/// attitude and thrust family, when the mapping is not exact, or when the
/// normalized value is not finite.
pub fn step_request(
    family: ControlFamily,
    channel: ControlChannel,
    mapping: StimulusMapping,
    envelope: &StimulusEnvelope,
    normalized: f64,
) -> Result<DirectStepRequest, AviateRuntimeError> {
    if family != ControlFamily::DirectAttitudeThrust {
        return Err(AviateRuntimeError::UnsupportedDirectFamily {
            family: family.as_str(),
        });
    }
    if mapping != StimulusMapping::AffineExact {
        return Err(AviateRuntimeError::InexactDirectMapping {
            mapping: mapping.as_str(),
        });
    }
    Ok(DirectStepRequest {
        family,
        channel,
        mapping,
        envelope: envelope.clone(),
        normalized: clamp_normalized("direct stimulus", normalized)?,
    })
}
