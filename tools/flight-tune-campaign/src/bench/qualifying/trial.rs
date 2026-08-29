//! The trials the bench flies, as stored documents and scoped limits.
//!
//! A trial becomes a mission document once. The reference names that document,
//! so the campaign schedule and the executed mission carry one identity, and
//! every scoped limit is keyed by the revision that document states.

use flight_tune::{AdapterError, Digest, MissionDocument, MissionReference};

use super::BenchVehicle;

/// The ceiling covers the whole trial at the bench's sample rate with room for
/// the completion event.
pub(super) const BENCH_MAX_SAMPLES: u32 = 700;

/// How long one sample of the bench may take to arrive.
///
/// The mission wall ceiling is this timeout for every permitted sample, so the
/// value prices the slowest sample a campaign can produce. The engine verifies
/// the complete durable journal once for each sample, and a campaign that
/// searches several suites writes more journal entries than one that searches
/// a single set, so the last runs of the longest campaign state the slowest
/// samples the bench has to allow.
const BENCH_RECEIPT_TIMEOUT_NS: u64 = 400_000_000;

const BENCH_COMPLETION_TIME_NS: u64 = 10_480_000_000;

/// The trial names that the bench stores as mission documents.
pub(super) const BENCH_TRIAL_IDS: [&str; 4] = [
    "training-step",
    "training-operator",
    BENCH_PROMOTION_TRIAL_ID,
    BENCH_FINAL_TRIAL_ID,
];

/// The hidden trial the one promotion decision is measured on.
pub const BENCH_PROMOTION_TRIAL_ID: &str = "promotion-step";
/// The hidden trial the final release decision is measured on.
pub const BENCH_FINAL_TRIAL_ID: &str = "final-step";

/// The normalized input the bench trial holds.
///
/// It is a FIRM input rather than a full-scale one, for the reason
/// `BenchBackend::input_at` states, and the two have to agree: the stimulus a
/// mission document commands is the stimulus the plant flies.
pub(super) const BENCH_STIMULUS_INPUT: f64 = 0.85;

/// Resolves the stored bench mission that one reference names.
///
/// # Errors
///
/// Returns an error when the bench stores no mission with that revision.
pub fn bench_stored_mission(
    mission: &MissionReference,
    model: BenchVehicle,
) -> Result<MissionDocument, AdapterError> {
    for id in BENCH_TRIAL_IDS {
        let document = bench_mission(id, model)?;
        if document.identity.revision_id == mission.revision_id {
            return Ok(document);
        }
    }
    Err(AdapterError::new(
        "the bench stores no mission document with that revision",
    ))
}

/// The revision identity one bench trial's stored mission carries.
///
/// A stored document names itself by revision rather than by the authored
/// trial name, and every scoped limit is keyed by that revision.
///
/// # Errors
///
/// Returns an error when the mission identity cannot be calculated.
pub fn bench_mission_revision_id(id: &str, model: BenchVehicle) -> Result<String, AdapterError> {
    Ok(bench_scenario(id, model)?.revision_id)
}

/// The physical envelope one bench vehicle commands.
///
/// The endpoints are the vehicle's own full-scale speed, so the normalized
/// range means a different physical demand on each aircraft and the two
/// vehicles cannot share one scoped limit by accident.
fn bench_envelope(model: BenchVehicle) -> flight_tune::StimulusEnvelope {
    flight_tune::StimulusEnvelope {
        id: "bench-operator-velocity".to_owned(),
        revision: 1,
        unit: flight_tune::PhysicalUnit::MetersPerSecond,
        reference: flight_tune::ReferenceRule::Zero,
        negative_endpoint: -model.full_scale_mps,
        neutral: 0.0,
        positive_endpoint: model.full_scale_mps,
    }
}

/// The identity of the envelope a bench scenario carries.
fn bench_envelope_digest(model: BenchVehicle) -> Result<Digest, AdapterError> {
    let digest = bench_envelope(model)
        .canonical_digest()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    Ok(Digest::from_bytes(*digest.as_bytes()))
}

/// The physical target one bench scenario asks the vehicle to reach.
///
/// It is the affine value of the held normalized input under the envelope,
/// which for an operator family is a bound rather than an exact command: the
/// candidate curve shapes the stick to at most this speed, and how much of it
/// the candidate keeps is what the authority band measures.
#[must_use]
pub fn bench_physical_target(model: BenchVehicle) -> f64 {
    model.full_scale_mps * BENCH_STIMULUS_INPUT
}

/// One trial of the bench, as its stored mission document.
fn bench_mission(id: &str, model: BenchVehicle) -> Result<MissionDocument, AdapterError> {
    let scenario = flight_tune::reference_stimulus_scenario(
        id,
        BENCH_COMPLETION_TIME_NS,
        &flight_tune::ReferenceStimulus {
            family: flight_tune::ControlFamily::OperatorVelocity,
            channel: flight_tune::ControlChannel::Roll,
            envelope: bench_envelope(model),
            normalized_value: BENCH_STIMULUS_INPUT,
        },
    )
    .map_err(|error| AdapterError::new(error.to_string()))?;
    flight_tune::calibration_mission_document(&scenario, 0, BENCH_RECEIPT_TIMEOUT_NS)
        .map_err(|error| AdapterError::new(error.to_string()))
}

/// One trial of the bench, as a mission reference.
///
/// # Errors
///
/// Returns an error when the mission identity cannot be calculated.
pub fn bench_scenario(id: &str, model: BenchVehicle) -> Result<MissionReference, AdapterError> {
    MissionReference::from_document(&bench_mission(id, model)?, BENCH_MAX_SAMPLES)
        .map_err(|error| AdapterError::new(error.to_string()))
}

/// The scoped response targets for one bench vehicle.
///
/// Every hidden decision the stage takes gets one row for each objective its
/// policy declares, in the scope the scenario actually commands: the operator
/// velocity family on the roll channel, at this vehicle's own envelope.
///
/// # Errors
///
/// Returns an error when a scenario identity or the table is not valid.
pub fn bench_response_targets(
    model: BenchVehicle,
    promotion_limits: &[(&str, f64)],
    qualification_limits: &[(&str, f64)],
    authority_band: flight_tune::TargetAuthorityBand,
) -> Result<flight_tune::ResponseTargetTable, AdapterError> {
    let envelope_digest = bench_envelope_digest(model)?;
    let mut rows = Vec::new();
    for (id, limits) in [
        (BENCH_PROMOTION_TRIAL_ID, promotion_limits),
        (BENCH_FINAL_TRIAL_ID, qualification_limits),
    ] {
        let mission = bench_scenario(id, model)?;
        let scope = flight_tune::ResponseTargetScope {
            mission_revision_id: mission.revision_id.clone(),
            mission_content_digest: mission.content_digest,
            control_family: flight_tune::ControlFamily::OperatorVelocity,
            control_channel: flight_tune::ControlChannel::Roll,
            physical_target: flight_tune::PhysicalTarget {
                unit: flight_tune::PhysicalUnit::MetersPerSecond,
                value: bench_physical_target(model),
            },
            envelope_digest,
            authority_band: Some(authority_band),
        };
        rows.extend(
            scope.rows(
                limits
                    .iter()
                    .map(|(name, limit)| (*name, flight_tune::TargetComparison::AtMost, *limit)),
            ),
        );
    }
    flight_tune::ResponseTargetTable::new(rows)
        .map_err(|error| AdapterError::new(error.to_string()))
}
