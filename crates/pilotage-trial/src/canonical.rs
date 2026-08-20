//! Canonical JSON and digest operations.

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as ShaDigest, Sha256};

use crate::{CodecError, Digest};

pub(crate) fn decode<T: DeserializeOwned>(
    document: &'static str,
    bytes: &[u8],
    limit: usize,
) -> Result<T, CodecError> {
    check_size(document, bytes.len(), limit)?;
    serde_json::from_slice(bytes).map_err(|source| CodecError::Decode { document, source })
}

pub(crate) fn encode<T: Serialize>(
    document: &'static str,
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, CodecError> {
    let bytes =
        serde_json::to_vec(value).map_err(|source| CodecError::Encode { document, source })?;
    check_size(document, bytes.len(), limit)?;
    Ok(bytes)
}

pub(crate) fn digest(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::from_bytes(hasher.finalize().into())
}

fn check_size(document: &'static str, size: usize, limit: usize) -> Result<(), CodecError> {
    if size <= limit {
        return Ok(());
    }
    Err(CodecError::DocumentTooLarge {
        document,
        size,
        limit,
    })
}
