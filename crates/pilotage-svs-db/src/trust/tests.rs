//! Tests for trust-root signature verification.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{TrustRoot, verify_signature};
use crate::canonical::manifest_canonical_bytes;
use crate::error::DbError;
use crate::fixtures;

#[test]
fn a_valid_signature_verifies_against_the_trust_root() {
    let candidate = fixtures::candidate();
    let bytes = manifest_canonical_bytes(&candidate.manifest);
    assert_eq!(
        verify_signature(
            &fixtures::trust_root(),
            &candidate.manifest.signature,
            &bytes
        ),
        Ok(())
    );
}

#[test]
fn a_key_id_absent_from_the_trust_root_is_refused() {
    let candidate = fixtures::candidate();
    let bytes = manifest_canonical_bytes(&candidate.manifest);
    let empty = TrustRoot::new(vec![]);
    assert_eq!(
        verify_signature(&empty, &candidate.manifest.signature, &bytes),
        Err(DbError::UntrustedRoot {
            key_id: candidate.manifest.signature.key_id,
        })
    );
}

#[test]
fn a_trusted_id_bound_to_the_wrong_key_fails_signature() {
    let candidate = fixtures::candidate();
    let bytes = manifest_canonical_bytes(&candidate.manifest);
    assert_eq!(
        verify_signature(
            &fixtures::wrong_key_trust_root(),
            &candidate.manifest.signature,
            &bytes,
        ),
        Err(DbError::SignatureInvalid)
    );
}

#[test]
fn a_mutated_message_fails_signature() {
    let candidate = fixtures::candidate();
    let mut bytes = manifest_canonical_bytes(&candidate.manifest);
    bytes[0] ^= 0x01;
    assert_eq!(
        verify_signature(
            &fixtures::trust_root(),
            &candidate.manifest.signature,
            &bytes
        ),
        Err(DbError::SignatureInvalid)
    );
}
