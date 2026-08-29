use crate::journal::storage;
use crate::{Candidate, Digest, JournalEvent, SearchStage, TuneError};

use super::{AuthenticatedJournalRecord, Journal, invalid};

/// Collects every candidate the chain names, in first-appearance order.
///
/// # Errors
///
/// Returns [`TuneError`] when a named candidate is not readable.
pub(super) fn from_journal(journal: &Journal) -> Result<Vec<Candidate>, TuneError> {
    let mut candidates = Vec::new();
    for digest in named_digests(journal.entries().iter().map(|entry| &entry.event)) {
        candidates.push(journal.read_candidate(digest)?);
    }
    Ok(candidates)
}

/// Checks the candidate list and every derived search group in the chain.
///
/// The list must state exactly the candidates the chain names and nothing
/// else. Each recorded transition must name the group that the difference
/// between its two exact candidates selects.
pub(super) fn validate(
    chain: &[AuthenticatedJournalRecord],
    candidates: &[Candidate],
    stage: &SearchStage,
) -> Result<(), TuneError> {
    let expected = named_digests(chain.iter().map(|record| &record.entry.event));
    if candidates.len() != expected.len() {
        return Err(invalid("the campaign candidate list is not complete"));
    }
    for (candidate, digest) in candidates.iter().zip(&expected) {
        candidate.validate()?;
        if storage::document_digest("candidate", candidate)? != *digest {
            return Err(invalid("a campaign candidate changed its identity"));
        }
    }
    for record in chain {
        let JournalEvent::CandidateTransitionAuthorized {
            candidate,
            group,
            receipt,
            ..
        } = &record.entry.event
        else {
            continue;
        };
        let source = find(candidates, &expected, receipt.source_candidate_digest())?;
        let target = find(candidates, &expected, *candidate)?;
        if &stage.derive_search_group(source, target)? != group {
            return Err(invalid(
                "a recorded transition suite does not match its candidate difference",
            ));
        }
    }
    Ok(())
}

fn named_digests<'a>(events: impl Iterator<Item = &'a JournalEvent>) -> Vec<Digest> {
    let mut digests = Vec::new();
    for event in events {
        let named = match event {
            JournalEvent::Started { candidate }
            | JournalEvent::CandidateTransitionAuthorized { candidate, .. } => *candidate,
            _ => continue,
        };
        if !digests.contains(&named) {
            digests.push(named);
        }
    }
    digests
}

fn find<'a>(
    candidates: &'a [Candidate],
    digests: &[Digest],
    wanted: Digest,
) -> Result<&'a Candidate, TuneError> {
    digests
        .iter()
        .position(|digest| *digest == wanted)
        .and_then(|index| candidates.get(index))
        .ok_or_else(|| invalid("a recorded transition names an absent candidate"))
}
