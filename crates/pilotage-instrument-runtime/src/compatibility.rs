//! Indicate conformance-corpus identity linked by this runtime.

/// Version of the Indicate scene-conformance corpus for this build.
pub const CORPUS_VERSION: u32 = 4;

/// SHA-256 digest of the Indicate scene-conformance corpus for this build.
pub const CORPUS_DIGEST: &str = "1fb8e6de2734ff7506843b05869f39d501f0926599636c6110a7e3b0c6e1625e";

/// Returns the linked conformance-corpus version.
pub const fn corpus_version() -> u32 {
    CORPUS_VERSION
}

/// Returns the linked conformance-corpus digest as lowercase hex.
pub const fn corpus_digest_hex() -> &'static str {
    CORPUS_DIGEST
}
