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

async fn build_with_store(store: Option<(PathBuf, PathBuf)>) -> Result<DevIdentity, HostError> {
    if let Some((cert, key)) = store.as_ref().filter(|paths| fresh(paths)) {
        match Identity::load_pemfiles(cert, key).await {
            Ok(identity) => return Ok(wrap(identity)),
            Err(error) => {
                tracing::warn!(%error, "persisted dev identity unreadable; regenerating");
            }
        }
    }

    let identity =
        Identity::self_signed(["localhost", "127.0.0.1", "::1"]).map_err(HostError::Identity)?;
    if let Some(paths) = store {
        persist(&identity, &paths);
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
/// requested or no home directory exists to persist under.
fn identity_store() -> Option<(PathBuf, PathBuf)> {
    if std::env::var_os("PILOTAGE_TLS_EPHEMERAL").is_some_and(|v| v == "1") {
        return None;
    }
    let dir = std::env::var_os("PILOTAGE_TLS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pilotage/dev-identity"))
        })?;
    Some((dir.join("dev-cert.pem"), dir.join("dev-key.pem")))
}

/// The issuance marker beside the PEM pair: the moment `persist` wrote
/// the identity, as seconds since the UNIX epoch. A file mtime is NOT
/// an issuance record — a copied or touched certificate reads young
/// while the certificate itself is past its validity, and the host
/// then serves an expired identity that fails every handshake.
fn issued_at_path(cert: &std::path::Path) -> PathBuf {
    cert.with_file_name("issued-at")
}

/// Whether the persisted certificate is young enough to reuse, judged
/// by the issuance marker written beside it. No marker means the pair
/// predates the marker scheme or was copied in: regenerate.
fn fresh((cert, _): &(PathBuf, PathBuf)) -> bool {
    std::fs::read_to_string(issued_at_path(cert))
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .zip(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok(),
        )
        .is_some_and(|(issued, now)| now.as_secs().saturating_sub(issued) < REUSE_LIMIT.as_secs())
}

/// Best-effort persistence, atomic per file: each PEM lands under a
/// temporary name and renames into place, so two hosts racing a cold
/// store cannot interleave one's certificate with the other's key —
/// a mismatched pair would reload cleanly on every later start and
/// fail every handshake until it aged out. Order still matters: the
/// key renames before the certificate, and the issuance marker lands
/// last, so a `fresh` certificate always has its key beside it.
fn persist(identity: &Identity, (cert_path, key_path): &(PathBuf, PathBuf)) {
    let Some(cert) = identity.certificate_chain().as_slice().first() else {
        return;
    };
    let write = || -> std::io::Result<()> {
        if let Some(dir) = cert_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let pid = std::process::id();
        let atomic = |path: &std::path::Path, bytes: &[u8], mode: Option<u32>| {
            let tmp = path.with_extension(format!("tmp.{pid}"));
            std::fs::write(&tmp, bytes)?;
            #[cfg(unix)]
            if let Some(mode) = mode {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
            }
            #[cfg(not(unix))]
            let _ = mode;
            std::fs::rename(&tmp, path)
        };
        atomic(
            key_path,
            identity.private_key().to_secret_pem().as_bytes(),
            Some(0o600),
        )?;
        atomic(cert_path, cert.to_pem().as_bytes(), None)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| std::io::Error::other("clock before the epoch"))?
            .as_secs();
        atomic(&issued_at_path(cert_path), now.to_string().as_bytes(), None)?;
        Ok(())
    };
    if let Err(error) = write() {
        tracing::warn!(%error, "could not persist the dev identity; it will rotate next run");
    }
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
    use super::{build_with_store, hex_encode};

    #[test]
    fn hex_encode_is_lowercase_no_separators() {
        assert_eq!(hex_encode(&[0xAB, 0x01, 0xFF]), "ab01ff");
    }

    #[tokio::test]
    async fn dev_identity_builds_and_hashes() {
        // No store: ephemeral, touching nothing on disk.
        let dev = build_with_store(None)
            .await
            .expect("self-signed identity builds");
        assert_eq!(dev.cert_hash_hex.len(), 64);
        assert!(dev.cert_hash_hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn the_identity_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("pilotage-tls-test-{}", std::process::id()));
        let store = Some((dir.join("dev-cert.pem"), dir.join("dev-key.pem")));
        let first = build_with_store(store.clone())
            .await
            .expect("first identity");
        let second = build_with_store(store).await.expect("second identity");
        assert_eq!(
            first.cert_hash_hex, second.cert_hash_hex,
            "a client's pinned link must survive a host restart"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
