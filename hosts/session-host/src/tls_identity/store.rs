use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::io::{self, Write};
use std::path::Path;
use wtransport::Identity;
use wtransport::tls::{Certificate, CertificateChain, PrivateKey, rustls};

pub(super) async fn load(pair: &Path) -> io::Result<Identity> {
    // Both PEM blocks must come from one read. Separate path opens can see
    // opposite sides of an atomic replacement, even when the path is identical.
    let snapshot = tokio::fs::read(pair).await?;
    decode(&snapshot)
}

fn decode(snapshot: &[u8]) -> io::Result<Identity> {
    let certificates = CertificateDer::pem_slice_iter(snapshot)
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    let key = PrivateKeyDer::from_pem_slice(snapshot).map_err(io::Error::other)?;
    let checked = rustls::sign::CertifiedKey::from_der(
        certificates.clone(),
        key.clone_key(),
        &rustls::crypto::ring::default_provider(),
    )
    .map_err(io::Error::other)?;
    checked.keys_match().map_err(io::Error::other)?;
    let certificates = certificates
        .into_iter()
        .map(|cert| Certificate::from_der(cert.to_vec()).map_err(io::Error::other))
        .collect::<Result<Vec<_>, _>>()?;
    let PrivateKeyDer::Pkcs8(key) = key else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "persisted development identity requires a PKCS#8 key",
        ));
    };
    Ok(Identity::new(
        CertificateChain::new(certificates),
        PrivateKey::from_der_pkcs8(key.secret_pkcs8_der().to_vec()),
    ))
}

pub(super) fn persist_blocking(identity: &Identity, pair: &Path) {
    let Some(cert) = identity.certificate_chain().as_slice().first() else {
        return;
    };
    let write = || -> io::Result<()> {
        let parent = pair
            .parent()
            .ok_or_else(|| io::Error::other("identity path has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let combined = format!(
            "{}{}",
            identity.private_key().to_secret_pem(),
            cert.to_pem()
        );
        atomic_blocking(pair, combined.as_bytes())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        atomic_blocking(&super::issued_at_path(pair), now.to_string().as_bytes())
    };
    if let Err(error) = write() {
        tracing::warn!(%error, path = %pair.display(), "could not persist the dev identity; it will rotate next run");
    }
}

fn atomic_blocking(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("identity path has no parent"))?;
    // Each writer needs its own inode, including concurrent tasks in one process.
    // NamedTempFile creates the secret with owner-only permissions before writing.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
