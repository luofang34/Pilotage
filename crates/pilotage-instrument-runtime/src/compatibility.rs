//! Indicate conformance-corpus identity linked by this runtime.

/// Version of the Indicate scene-conformance corpus for this build.
pub const CORPUS_VERSION: u32 = 9;

/// SHA-256 digest of the Indicate scene-conformance corpus for this build.
pub const CORPUS_DIGEST: &str = "dcd67f6cc789bb17199caeb364a67fbaaa211f32de5353d50eaffa989e846b2b";

/// Returns the linked conformance-corpus version.
pub const fn corpus_version() -> u32 {
    CORPUS_VERSION
}

/// Returns the linked conformance-corpus digest as lowercase hex.
pub const fn corpus_digest_hex() -> &'static str {
    CORPUS_DIGEST
}
