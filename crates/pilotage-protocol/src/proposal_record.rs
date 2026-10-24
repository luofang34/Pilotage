//! A mission proposal as immutable bytes (ADR-0014, ADR-0052). The lane that
//! compiles a proposal encodes it once; that body and the digest over it
//! travel together as a record. A receiver checks the digest against the
//! bytes it received and decodes them for display and review, but never
//! hashes a new encoding of the decoded message: protobuf encoding is not
//! canonical, so two encoders may write one proposal differently, and the
//! approval must name exactly what was reviewed.

use prost::Message as _;
use sha2::{Digest as _, Sha256};

use crate::wire;

#[cfg(test)]
mod tests;

/// The domain tag the digest of a record's body starts with.
const DOMAIN: &[u8] = b"pilotage.mission-proposal.v1\0";

/// The execution feature of a survey line task.
pub const FEATURE_SURVEY_LINE: &str = "pilotage.task.survey-line/1";
/// The execution feature of a return home task.
pub const FEATURE_RETURN_HOME: &str = "pilotage.task.return-home/1";
/// The execution feature of a team proposal that binds member records.
pub const FEATURE_TEAM: &str = "pilotage.team-of-lanes/1";

/// Why a record is refused.
#[derive(Debug, thiserror::Error)]
pub enum RecordRefused {
    /// The record's format is not one this receiver reads.
    #[error("the proposal record has format {0}, which this receiver does not read")]
    UnknownFormat(i32),
    /// The digest does not name the body's bytes.
    #[error("the proposal record's digest does not match its body")]
    DigestMismatch,
    /// The body is not a proposal.
    #[error("the proposal record's body does not decode")]
    Undecodable(#[source] prost::DecodeError),
}

/// A proposal and the record that carries it. The decoded proposal's
/// `digest` holds the record's digest, for the code that reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct SealedProposal {
    proposal: wire::MissionProposal,
    record: wire::MissionProposalRecord,
}

/// The digest of a record body.
#[must_use]
pub fn body_digest(body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(body);
    hasher.finalize().into()
}

impl SealedProposal {
    /// Encodes `proposal` once, with its `digest` empty, into a record.
    #[must_use]
    pub fn seal(mut proposal: wire::MissionProposal) -> Self {
        proposal.digest.clear();
        let body = proposal.encode_to_vec();
        let digest = body_digest(&body).to_vec();
        proposal.digest.clone_from(&digest);
        Self {
            proposal,
            record: wire::MissionProposalRecord {
                format: wire::MissionProposalFormat::V1 as i32,
                body,
                digest,
            },
        }
    }

    /// Opens a received record: its format is known, its digest names its
    /// body, and the body decodes.
    ///
    /// # Errors
    ///
    /// [`RecordRefused`] for an unknown format, a digest that does not match
    /// the body, or a body that does not decode.
    pub fn open(record: wire::MissionProposalRecord) -> Result<Self, RecordRefused> {
        if record.format != wire::MissionProposalFormat::V1 as i32 {
            return Err(RecordRefused::UnknownFormat(record.format));
        }
        if body_digest(&record.body).as_slice() != record.digest.as_slice() {
            return Err(RecordRefused::DigestMismatch);
        }
        let mut proposal = wire::MissionProposal::decode(record.body.as_slice())
            .map_err(RecordRefused::Undecodable)?;
        proposal.digest.clone_from(&record.digest);
        Ok(Self { proposal, record })
    }

    /// The decoded proposal, for display and review.
    #[must_use]
    pub fn proposal(&self) -> &wire::MissionProposal {
        &self.proposal
    }

    /// The record, as it travels.
    #[must_use]
    pub fn record(&self) -> &wire::MissionProposalRecord {
        &self.record
    }

    /// The digest an approval binds.
    #[must_use]
    pub fn digest(&self) -> &[u8] {
        &self.record.digest
    }

    /// The decoded proposal and the record.
    #[must_use]
    pub fn into_parts(self) -> (wire::MissionProposal, wire::MissionProposalRecord) {
        (self.proposal, self.record)
    }
}

/// The features `proposal` requires that are not in `known`.
#[must_use]
pub fn unknown_features(proposal: &wire::MissionProposal, known: &[&str]) -> Vec<String> {
    proposal
        .requires
        .iter()
        .filter(|f| !known.contains(&f.as_str()))
        .cloned()
        .collect()
}
