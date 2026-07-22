//! The canonical content digest of a device profile (ADR-0009).
//!
//! A profile's monotonic `revision` says *which generation* of a profile
//! produced a control frame; the content digest says *exactly which bytes*.
//! Binding both to a frame lets a receiver prove the frame was produced by the
//! precise profile it holds, not merely one claiming the same revision.
//!
//! The algorithm lives here, in one place, so the native host and the WASM
//! browser engine compute an identical digest for identical bytes — a digest
//! that disagreed across the two builds could never bind evidence.

use sha2::{Digest, Sha256};

/// The number of bytes in a profile content digest.
pub const DIGEST_LEN: usize = 32;

/// The SHA-256 content digest of a profile's on-the-wire bytes.
///
/// Taken over the exact bytes a caller loaded (file, cache, or server
/// response), so two byte-identical profiles share a digest and any single-byte
/// difference changes it. Callers that need whitespace-independence re-serialize
/// the parsed [`crate::DeviceProfile`] canonically before hashing.
#[must_use]
pub fn content_digest(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DIGEST_LEN, content_digest};

    #[test]
    fn digest_is_stable_across_calls() {
        let bytes = br#"{"schema_version":1}"#;
        assert_eq!(content_digest(bytes), content_digest(bytes));
    }

    #[test]
    fn a_single_byte_change_changes_the_digest() {
        let a = content_digest(b"profile-a");
        let b = content_digest(b"profile-b");
        assert_ne!(a, b, "the digest must be sensitive to content");
    }

    #[test]
    fn digest_is_thirty_two_bytes() {
        assert_eq!(content_digest(b"").len(), DIGEST_LEN);
    }
}
