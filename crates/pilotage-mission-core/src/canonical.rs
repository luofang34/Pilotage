//! Canonical JSON and domain-separated digest operations.

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as ShaDigest, Sha256};

use crate::{
    CodecError, Digest, ExecutionPolicy, MAX_DOCUMENT_BYTES, MissionDocument, MissionPhase,
    NavigationDataIdentity,
};

const CONTENT_DIGEST_DOMAIN: &[u8] = b"pilotage.mission-document.content.v1\0";

#[derive(Serialize)]
struct ContentIdentity<'a> {
    revision_id: &'a str,
    schema_version: u16,
    navigation_data_identity: &'a NavigationDataIdentity,
}

#[derive(Serialize)]
struct MissionContent<'a> {
    identity: ContentIdentity<'a>,
    execution_policy: &'a ExecutionPolicy,
    phases: &'a [MissionPhase],
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    check_size(bytes.len())?;
    serde_json::from_slice(bytes).map_err(|source| CodecError::Decode { source })
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    let bytes = serde_json::to_vec(value).map_err(|source| CodecError::Encode { source })?;
    check_size(bytes.len())?;
    Ok(bytes)
}

pub(crate) fn content_digest(document: &MissionDocument) -> Result<Digest, CodecError> {
    let content = MissionContent {
        identity: ContentIdentity {
            revision_id: &document.identity.revision_id,
            schema_version: document.identity.schema_version,
            navigation_data_identity: &document.identity.navigation_data_identity,
        },
        execution_policy: &document.execution_policy,
        phases: &document.phases,
    };
    let bytes = encode(&content)?;
    let mut hasher = Sha256::new();
    hasher.update(CONTENT_DIGEST_DOMAIN);
    hasher.update(bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

fn check_size(size: usize) -> Result<(), CodecError> {
    if size <= MAX_DOCUMENT_BYTES {
        return Ok(());
    }
    Err(CodecError::DocumentTooLarge {
        size,
        limit: MAX_DOCUMENT_BYTES,
    })
}
