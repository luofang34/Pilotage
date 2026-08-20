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

async fn build_with_store(store: Option<PathBuf>) -> Result<DevIdentity, HostError> {
    if let Some(pair) = store.as_ref().filter(|pair| fresh(pair)) {
        // The SAME file serves as both arguments: the PEM readers filter
        // by block type, so the certificate loader takes the CERTIFICATE
        // block and the key loader the PRIVATE KEY block. Keeping the
        // pair in one file is what makes it a pair — a rename replaces
        // both together, so no reader can ever see one identity's
        // certificate beside another's key.
        match Identity::load_pemfiles(pair, pair).await {
            Ok(identity) => return Ok(wrap(identity)),
            Err(error) => {
                tracing::warn!(%error, "persisted dev identity unreadable; regenerating");
            }
        }
    }

    let identity =
        Identity::self_signed(["localhost", "127.0.0.1", "::1"]).map_err(HostError::Identity)?;
    if let Some(pair) = store {
        persist(&identity, &pair);
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

/// Best-effort persistence. The key and certificate blocks land in ONE
/// file through one atomic rename: per-file atomicity over a split pair
/// would still let two hosts racing a cold store interleave one's
/// certificate with the other's key, and rustls refuses a mismatched
/// pair at server-config time — after loading reported success. With a
/// single file the last writer wins whole. The issuance marker lands
/// after the pair, so a `fresh` pair is always complete; the file is
/// owner-only, since it carries the secret key.
fn persist(identity: &Identity, pair: &std::path::Path) {
    let Some(cert) = identity.certificate_chain().as_slice().first() else {
        return;
    };
    let write = || -> std::io::Result<()> {
        if let Some(dir) = pair.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let pid = std::process::id();
        let atomic = |path: &std::path::Path, bytes: &[u8], secret: bool| {
            let tmp = path.with_extension(format!("tmp.{pid}"));
            std::fs::write(&tmp, bytes)?;
            #[cfg(unix)]
            if secret {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
            #[cfg(not(unix))]
            let _ = secret;
            std::fs::rename(&tmp, path)
        };
        let combined = format!(
            "{}{}",
            identity.private_key().to_secret_pem(),
            cert.to_pem()
        );
        atomic(pair, combined.as_bytes(), true)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| std::io::Error::other("clock before the epoch"))?
            .as_secs();
        atomic(&issued_at_path(pair), now.to_string().as_bytes(), false)?;
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
        let store = Some(dir.join("dev-identity.pem"));
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn racing_cold_stores_never_leave_a_torn_pair() {
        // The failure this pins: concurrent builds against a cold store
        // interleaving one identity's certificate with another's key. A
        // torn pair LOADS cleanly — the readers do not cross-check the
        // blocks — and then panics inside the TLS stack at server-config
        // time. So the racers must run on real parallel threads, and the
        // settled identity must survive the server-config build itself,
        // the exact point a torn pair detonates.
        let dir = std::env::temp_dir().join(format!(
            "pilotage-tls-race-{}-{}",
            std::process::id(),
            line!()
        ));
        let store = dir.join("dev-identity.pem");
        let racers: Vec<_> = (0..8)
            .map(|_| {
                let store = Some(store.clone());
                tokio::spawn(async move {
                    build_with_store(store)
                        .await
                        .expect("a racing build must not fail")
                        .cert_hash_hex
                })
            })
            .collect();
        for racer in racers {
            racer.await.expect("racer task completes");
        }
        let settled = build_with_store(Some(store.clone()))
            .await
            .expect("the settled store loads");
        let reloaded = build_with_store(Some(store))
            .await
            .expect("the settled store loads again");
        assert_eq!(
            settled.cert_hash_hex, reloaded.cert_hash_hex,
            "the store must converge on one whole identity"
        );
        let server_config = wtransport::ServerConfig::builder()
            .with_bind_address(([127, 0, 0, 1], 0).into())
            .with_identity(settled.identity)
            .build();
        drop(server_config);
        std::fs::remove_dir_all(&dir).ok();
    }
}
