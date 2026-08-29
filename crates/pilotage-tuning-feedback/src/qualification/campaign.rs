use flight_tune::{Digest, JournalEvidenceSnapshot, RuntimeIdentities, SessionIdentity};

use crate::{FeedbackError, digest, error::invalid};

use super::stage;
use super::{authority, authority::VerifiedAuthority};

pub(super) mod retry;

const JOURNAL_SCHEMA_VERSION: u32 = 8;
const SNAPSHOT_SCHEMA_VERSION: u16 = 5;

pub(super) struct CampaignIdentity {
    pub(super) session_digest: Digest,
    pub(super) authority: VerifiedAuthority,
}

pub(super) fn verify(
    snapshot: &JournalEvidenceSnapshot,
) -> Result<CampaignIdentity, FeedbackError> {
    if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(invalid("the journal evidence snapshot schema changed"));
    }
    stage::verify(&snapshot.stage)?;
    let entry = &snapshot.head.entry;
    if entry.schema_version != JOURNAL_SCHEMA_VERSION
        || entry.sequence == 0
        || entry.previous.is_none_or(Digest::is_zero)
        || snapshot.head.entry_digest.is_zero()
        || snapshot.head.entry_digest != digest::document("journal entry", entry)?
    {
        return Err(invalid("the authenticated journal head changed"));
    }
    verify_session(&entry.session, &snapshot.stage)?;
    let session_digest = digest::document("session identity", &entry.session)?;
    let authority = authority::verify(snapshot)?;
    Ok(CampaignIdentity {
        session_digest,
        authority,
    })
}

fn verify_session(
    session: &SessionIdentity,
    stage_value: &flight_tune::SearchStage,
) -> Result<(), FeedbackError> {
    if session.stage_digest.is_zero()
        || session.stage_digest != digest::document("search stage", stage_value)?
        || session.initial_candidate_digest.is_zero()
        || session.candidate_lineage.schema.trim().is_empty()
        || session.candidate_lineage.schema.len() > 128
        || session.candidate_lineage.base_preset_digest.is_zero()
        || session.candidate_lineage.plant_digest.is_zero()
        || session.runtimes.adjacency_policy_digest.is_zero()
    {
        return Err(invalid("the tuning session identity is not valid"));
    }
    verify_runtimes(&session.runtimes)
}

fn verify_runtimes(runtimes: &RuntimeIdentities) -> Result<(), FeedbackError> {
    for (artifact, name) in [
        (&runtimes.harness_build, "harness build"),
        (&runtimes.strategy, "strategy"),
        (&runtimes.metric, "metric"),
        (&runtimes.hard_gates, "hard gates"),
        (&runtimes.simulator, "simulator"),
        (&runtimes.airframe, "airframe"),
        (&runtimes.vehicle, "vehicle"),
        (&runtimes.transition_validator, "transition validator"),
    ] {
        stage::verify_artifact(artifact, name)?;
    }
    Ok(())
}
