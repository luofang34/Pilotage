//! The candidate list the verifier checks before it reads any parameter.
//!
//! A search group follows from the parameters that differ between two exact
//! candidates. The chain carries digests, so the candidates travel with the
//! evidence. Every one of them is recomputed here before it answers anything.

use flight_tune::{AuthenticatedJournalRecord, Candidate, Digest, JournalEvent};

use crate::{FeedbackError, digest, error::invalid};

/// The candidates the chain names, checked against their own identities.
pub(super) struct VerifiedCandidates<'a> {
    digests: Vec<Digest>,
    candidates: &'a [Candidate],
}

impl VerifiedCandidates<'_> {
    /// Returns the exact candidate one digest names.
    pub(super) fn get(&self, wanted: Digest) -> Result<&Candidate, FeedbackError> {
        self.digests
            .iter()
            .position(|held| *held == wanted)
            .and_then(|index| self.candidates.get(index))
            .ok_or_else(|| invalid("the campaign evidence has no candidate for a chain digest"))
    }
}

/// Checks that the candidate list states exactly what the chain names.
pub(super) fn verify<'a>(
    chain: &[AuthenticatedJournalRecord],
    candidates: &'a [Candidate],
) -> Result<VerifiedCandidates<'a>, FeedbackError> {
    let digests = named_digests(chain);
    if candidates.len() != digests.len() {
        return Err(invalid("the campaign candidate list is not complete"));
    }
    for (candidate, expected) in candidates.iter().zip(&digests) {
        if digest::document("candidate", candidate)? != *expected {
            return Err(invalid("a campaign candidate changed its identity"));
        }
    }
    Ok(VerifiedCandidates {
        digests,
        candidates,
    })
}

fn named_digests(chain: &[AuthenticatedJournalRecord]) -> Vec<Digest> {
    let mut digests = Vec::new();
    for record in chain {
        let named = match record.entry.event {
            JournalEvent::Started { candidate }
            | JournalEvent::CandidateTransitionAuthorized { candidate, .. } => candidate,
            _ => continue,
        };
        if !digests.contains(&named) {
            digests.push(named);
        }
    }
    digests
}
