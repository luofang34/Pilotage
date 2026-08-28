use flight_tune::Digest;
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use crate::FeedbackError;

pub(crate) fn hash(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::from_bytes(hasher.finalize().into())
}

pub(crate) fn document<T: Serialize>(
    name: &'static str,
    value: &T,
) -> Result<Digest, FeedbackError> {
    let bytes = encode(name, value)?;
    Ok(hash(&bytes))
}

pub(crate) fn domain<T: Serialize>(
    name: &'static str,
    domain: &[u8],
    value: &T,
) -> Result<Digest, FeedbackError> {
    let document = encode(name, value)?;
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(document.len()));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&document);
    Ok(hash(&bytes))
}

pub(crate) fn encode<T: Serialize>(
    document: &'static str,
    value: &T,
) -> Result<Vec<u8>, FeedbackError> {
    serde_json::to_vec(value).map_err(|source| FeedbackError::Encode { document, source })
}
