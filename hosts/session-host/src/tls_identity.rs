//! Loopback-dev TLS identity: a self-signed certificate plus its SHA-256
//! digest, printed at startup so a local client can pin it out of band
//! (ADR-0005's local-demo certificate strategy).

use wtransport::Identity;
use wtransport::tls::Sha256Digest;

use crate::error::HostError;

/// A self-signed identity for `127.0.0.1`/`localhost`, plus the hex-encoded
/// SHA-256 digest of its leaf certificate for out-of-band client pinning.
pub struct DevIdentity {
    /// The TLS identity handed to [`wtransport::ServerConfig::builder`].
    pub identity: Identity,
    /// Lowercase hex SHA-256 digest of the leaf certificate, with no
    /// separators, suitable for the `LISTENING` machine-readable line.
    pub cert_hash_hex: String,
}

/// Builds a fresh self-signed identity valid for `localhost`, `127.0.0.1`,
/// and `::1`.
///
/// # Errors
///
/// Returns [`HostError::Identity`] if the subject alternative names above are
/// somehow not valid DNS `IA5` strings, which cannot happen for this fixed,
/// hard-coded set but is surfaced rather than unwrapped per the workspace's
/// no-`unwrap` policy.
pub fn build_dev_identity() -> Result<DevIdentity, HostError> {
    let identity =
        Identity::self_signed(["localhost", "127.0.0.1", "::1"]).map_err(HostError::Identity)?;
    let leaf = identity
        .certificate_chain()
        .as_slice()
        .first()
        .map_or_else(|| Sha256Digest::new([0; 32]), |cert| cert.hash());
    let cert_hash_hex = hex_encode(leaf.as_ref());
    Ok(DevIdentity {
        identity,
        cert_hash_hex,
    })
}

/// Encodes bytes as lowercase hex with no separators.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{build_dev_identity, hex_encode};

    #[test]
    fn hex_encode_is_lowercase_no_separators() {
        assert_eq!(hex_encode(&[0xAB, 0x01, 0xFF]), "ab01ff");
    }

    #[test]
    fn dev_identity_builds_and_hashes() {
        let dev = build_dev_identity().expect("self-signed identity builds");
        assert_eq!(dev.cert_hash_hex.len(), 64);
        assert!(dev.cert_hash_hex.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
