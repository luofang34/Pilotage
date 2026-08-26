use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use crate::{Digest, TuneError};

pub(super) fn domain_digest(
    domain: &[u8],
    document: &impl Serialize,
    name: &'static str,
) -> Result<Digest, TuneError> {
    let bytes = serde_json::to_vec(document).map_err(|source| TuneError::Encode {
        document: name,
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

pub(super) fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}
