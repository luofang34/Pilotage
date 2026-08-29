use pilotage_trial::Digest;
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use crate::identity::digest_bytes;
use crate::{ArtifactIdentity, Candidate, SearchGroupBinding, TuneError};

const RECEIPT_DOMAIN: &[u8] = b"pilotage.flight-tune.candidate-transition-receipt.v1\0";
const PLANNING_CONTEXT_DOMAIN: &[u8] =
    b"pilotage.flight-tune.candidate-transition-planning-context.v2\0";

#[derive(Serialize)]
pub(super) struct ReceiptDocument<'a> {
    pub(super) schema_version: u16,
    pub(super) session_digest: Digest,
    pub(super) source_candidate_digest: Digest,
    pub(super) target_candidate_digest: Digest,
    pub(super) validator: &'a ArtifactIdentity,
    pub(super) adjacency_policy_digest: Digest,
    pub(super) planning_context_digest: Digest,
}

#[derive(Serialize)]
struct PlanningContextDocument<'a> {
    schema_version: u16,
    stage_digest: Digest,
    plan_digest: Digest,
    group: &'a SearchGroupBinding,
}

pub(super) fn validate_candidate_digest(
    candidate: &Candidate,
    expected: Digest,
    role: &'static str,
) -> Result<(), TuneError> {
    let bytes = serde_json::to_vec(candidate).map_err(|source| TuneError::Encode {
        document: "candidate transition candidate",
        source,
    })?;
    if digest_bytes(&bytes) != expected {
        return Err(TuneError::InvalidIdentity {
            detail: format!("the {role} candidate digest does not match its exact candidate"),
        });
    }
    Ok(())
}

pub(super) fn receipt_digest(document: &ReceiptDocument<'_>) -> Result<Digest, TuneError> {
    domain_digest(RECEIPT_DOMAIN, document, "candidate transition receipt")
}

pub(crate) fn planning_context_digest(
    stage_digest: Digest,
    plan_digest: Digest,
    group: &SearchGroupBinding,
) -> Result<Digest, TuneError> {
    if stage_digest.is_zero()
        || plan_digest.is_zero()
        || group.suite_digest.is_zero()
        || group.group_id.trim().is_empty()
        || group.suite_id.trim().is_empty()
    {
        return Err(TuneError::InvalidIdentity {
            detail: "the candidate transition planning context is incomplete".to_owned(),
        });
    }
    domain_digest(
        PLANNING_CONTEXT_DOMAIN,
        &PlanningContextDocument {
            schema_version: 2,
            stage_digest,
            plan_digest,
            group,
        },
        "candidate transition planning context",
    )
}

fn domain_digest(
    domain: &[u8],
    document: &impl Serialize,
    name: &'static str,
) -> Result<Digest, TuneError> {
    let bytes = serde_json::to_vec(document).map_err(|source| TuneError::Encode {
        document: name,
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}
