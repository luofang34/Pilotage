use flight_tune::{Digest, FinalQualificationOutcome};

use crate::evidence::{CAMPAIGN_EVIDENCE_SCHEMA_VERSION, CampaignEvidence};
use crate::{FeedbackError, error::invalid};

mod authority;
mod campaign;
mod evaluation;
mod final_qualification;
mod plan;
mod promotion;
mod stage;
mod statistics;
mod training_suite;

#[cfg(test)]
mod tests;

struct Verification {
    selected_candidate: Option<Digest>,
    outcome: Option<FinalQualificationOutcome>,
}

pub(crate) fn verify(evidence: &CampaignEvidence) -> Result<(), FeedbackError> {
    verify_all(evidence).map(|_| ())
}

pub(crate) fn verify_qualified(evidence: &CampaignEvidence) -> Result<Digest, FeedbackError> {
    let verified = verify_all(evidence)?;
    if verified.outcome != Some(FinalQualificationOutcome::Qualified) {
        return Err(invalid("the campaign final result is not qualified"));
    }
    verified
        .selected_candidate
        .ok_or_else(|| invalid("qualified evidence has no selected candidate"))
}

fn verify_all(evidence: &CampaignEvidence) -> Result<Verification, FeedbackError> {
    if evidence.schema_version != CAMPAIGN_EVIDENCE_SCHEMA_VERSION {
        return Err(invalid("the campaign evidence schema changed"));
    }
    let identity = campaign::verify(&evidence.journal)?;
    let session = &evidence.journal.head.entry.session;
    let promotion = promotion::verify(&evidence.journal, session, &identity)?;
    let final_result =
        final_qualification::verify(&evidence.journal, session, &identity, &promotion)?;
    Ok(Verification {
        selected_candidate: promotion.selected_candidate,
        outcome: final_result.outcome,
    })
}
