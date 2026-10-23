//! The proposal digest: SHA-256 over the proposal's protobuf encoding with
//! the digest field empty. Prost encodes fields in tag order, so the same
//! proposal always gives the same bytes.

use pilotage_protocol::wire;
use prost::Message;
use sha2::{Digest as _, Sha256};

const DOMAIN: &[u8] = b"pilotage.mission-proposal.v1\0";

/// The digest an approval binds.
#[must_use]
pub fn proposal_digest(proposal: &wire::MissionProposal) -> [u8; 32] {
    let mut unsigned = proposal.clone();
    unsigned.digest.clear();
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(unsigned.encode_to_vec());
    hasher.finalize().into()
}
