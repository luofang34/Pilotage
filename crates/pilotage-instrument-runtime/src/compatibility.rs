//! Indicate conformance-corpus identity linked by this runtime.

/// Version of the Indicate scene-conformance corpus for this build.
pub const CORPUS_VERSION: u32 = 8;

/// SHA-256 digest of the Indicate scene-conformance corpus for this build.
pub const CORPUS_DIGEST: &str = "0b0c7ccb135bfc4107bc110e4b24dceffd84adf1b767fcd14d2c5ace7391f962";

/// Returns the linked conformance-corpus version.
pub const fn corpus_version() -> u32 {
    CORPUS_VERSION
}

/// Returns the linked conformance-corpus digest as lowercase hex.
pub const fn corpus_digest_hex() -> &'static str {
    CORPUS_DIGEST
}
