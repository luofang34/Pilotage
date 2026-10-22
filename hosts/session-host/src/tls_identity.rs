//! Loopback-dev TLS identity: a self-signed certificate plus its SHA-256
//! digest, printed at startup so a local client can pin it out of band
//! (ADR-0005's local-demo certificate strategy).
//!
//! The identity PERSISTS across host restarts. A client pins the
//! certificate by hash, so every fresh certificate invalidates every
//! link, bookmark, and open tab that carries the old one — a tab left
//! open across a host restart then retries a dead hash forever. The
//! identity is reused until it approaches the 14-day validity ceiling
//! WebTransport imposes on hash-pinned certificates, and only then
//! regenerated. `PILOTAGE_TLS_EPHEMERAL=1` restores the fresh-per-run
//! behavior; `PILOTAGE_TLS_DIR` overrides where the identity lives.

use std::path::PathBuf;
use std::time::Duration;

use wtransport::Identity;
use wtransport::tls::Sha256Digest;

use crate::error::HostError;

mod store;

/// How old a persisted identity may grow before it is regenerated.
/// Self-signed identities are valid 14 days; regenerating at 10 leaves
/// margin for clocks and long-running sessions.
const REUSE_LIMIT: Duration = Duration::from_secs(10 * 24 * 60 * 60);

/// A self-signed identity for `127.0.0.1`/`localhost`, plus the hex-encoded
/// SHA-256 digest of its leaf certificate for out-of-band client pinning.
pub struct DevIdentity {
    /// The TLS identity handed to [`wtransport::ServerConfig::builder`].
    pub identity: Identity,
    /// Lowercase hex SHA-256 digest of the leaf certificate, with no
    /// separators, suitable for the `LISTENING` machine-readable line.
    pub cert_hash_hex: String,
}

/// Loads the persisted dev identity when one is fresh enough, otherwise
/// builds a fresh self-signed identity valid for `localhost`,
/// `127.0.0.1`, and `::1` and persists it for the next run.
///
/// # Errors
///
/// Returns [`HostError::Identity`] if the subject alternative names above are
/// somehow not valid DNS `IA5` strings, which cannot happen for this fixed,
/// hard-coded set but is surfaced rather than unwrapped per the workspace's
/// no-`unwrap` policy. Persistence failures are logged and never fatal: a
/// host that cannot save its identity still serves, it just rotates.
pub async fn build_dev_identity() -> Result<DevIdentity, HostError> {
    build_with_store(identity_store()).await
}

async fn build_with_store(store: Option<PathBuf>) -> Result<DevIdentity, HostError> {
    if let Some(pair) = store.as_ref().filter(|pair| fresh(pair)) {
        match store::load(pair).await {
            Ok(identity) => return Ok(wrap(identity)),
            Err(error) => {
                tracing::warn!(%error, path = %pair.display(), "persisted dev identity unreadable; regenerating");
            }
        }
    }

    let identity =
        Identity::self_signed(["localhost", "127.0.0.1", "::1"]).map_err(HostError::Identity)?;
    if let Some(pair) = store {
        store::persist_blocking(&identity, &pair);
    }
    Ok(wrap(identity))
}

fn wrap(identity: Identity) -> DevIdentity {
    let leaf = identity
        .certificate_chain()
        .as_slice()
        .first()
        .map_or_else(|| Sha256Digest::new([0; 32]), |cert| cert.hash());
    let cert_hash_hex = hex_encode(leaf.as_ref());
    DevIdentity {
        identity,
        cert_hash_hex,
    }
}

/// Where the identity persists: `PILOTAGE_TLS_DIR`, else
/// `~/.pilotage/dev-identity`. `None` when ephemeral identities are
/// requested or no home directory exists to persist under. The pair
/// lives in ONE file — certificate and key blocks together — so the
/// atomic rename that lands it replaces the pair as a unit.
fn identity_store() -> Option<PathBuf> {
    if std::env::var_os("PILOTAGE_TLS_EPHEMERAL").is_some_and(|v| v == "1") {
        return None;
    }
    let dir = std::env::var_os("PILOTAGE_TLS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pilotage/dev-identity"))
        })?;
    Some(dir.join("dev-identity.pem"))
}

/// The issuance marker beside the PEM pair: the moment `persist` wrote
/// the identity, as seconds since the UNIX epoch. A file mtime is NOT
/// an issuance record — a copied or touched certificate reads young
/// while the certificate itself is past its validity, and the host
/// then serves an expired identity that fails every handshake.
fn issued_at_path(pair: &std::path::Path) -> PathBuf {
    pair.with_file_name("issued-at")
}

/// Whether the persisted certificate is young enough to reuse, judged
/// by the issuance marker written beside it. No marker means the pair
/// was copied in without one: regenerate.
fn fresh(pair: &std::path::Path) -> bool {
    std::fs::read_to_string(issued_at_path(pair))
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .zip(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok(),
        )
        .is_some_and(|(issued, now)| now.as_secs().saturating_sub(issued) < REUSE_LIMIT.as_secs())
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
mod tests;
