//! The complete production-input identity of the Aviate scenario runtime.
//!
//! The build script inventories every production runtime source, writes one
//! canonical schema-versioned document, and embeds that document with its
//! digest. This module reads the document back, recomputes the digest from
//! the embedded bytes, and binds the result to the vehicle, transition
//! validator, adjacency policy, direct transport, and runtime configuration.
//!
//! One value comes out: the runtime implementation identity. A production
//! input that changes changes it. A test input cannot enter it, because the
//! inventory refuses a test path. Ordering cannot change it, because the
//! canonical document sorts its entries and the digest frames each length.
//!
//! SIM / NOT FOR FLIGHT.

use std::collections::BTreeSet;

use flight_tune::{ArtifactIdentity, Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::AviateRuntimeError;

include!(concat!(env!("OUT_DIR"), "/runtime_source_identity.rs"));

/// The supported runtime implementation document schema.
pub const RUNTIME_IMPLEMENTATION_SCHEMA_VERSION: u16 = 1;

/// The stable name of the Aviate runtime implementation identity.
pub const RUNTIME_IMPLEMENTATION_ID: &str = "pilotage-aviate-runtime-implementation-v1";

/// The schema version of the embedded source-inventory document.
const SOURCE_DOCUMENT_SCHEMA_VERSION: u16 = 1;

/// The domain the build script separates the source document with.
///
/// The value has to match the build script byte for byte. A readback that
/// recomputes the digest under another domain would accept a document the
/// build never wrote.
const SOURCE_DOCUMENT_DOMAIN: &[u8] = b"flight-tune-aviate-runtime-source-document-v1\0";

/// The domain that separates the runtime implementation document.
const IMPLEMENTATION_DOMAIN: &[u8] = b"pilotage-aviate-runtime-implementation-v1\0";

/// One production runtime source and its exact content identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSourceEntry {
    /// The package-relative production source path.
    pub path: String,
    /// The lowercase hexadecimal SHA-256 of the source bytes.
    pub sha256: String,
    /// The source length in bytes.
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSourceDocument {
    schema_version: u16,
    entries: Vec<RuntimeSourceEntry>,
}

/// The exact non-source identities that shape one Aviate runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeIdentityInputs {
    /// The exact vehicle implementation and configuration identity.
    pub vehicle: ArtifactIdentity,
    /// The exact candidate-transition validator identity.
    pub transition_validator: ArtifactIdentity,
    /// The exact vehicle adjacency-policy identity.
    pub adjacency_policy_digest: Digest,
    /// The exact simulator-only direct-transport implementation identity.
    pub direct_transport: ArtifactIdentity,
    /// The exact runtime configuration identity.
    pub configuration: ArtifactIdentity,
}

/// The canonical runtime implementation document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeImplementationDocument {
    /// The document schema version.
    pub schema_version: u16,
    /// The embedded source-inventory document identity.
    pub source_document_digest: Digest,
    /// Every production runtime source in canonical path order.
    pub sources: Vec<RuntimeSourceEntry>,
    /// The exact vehicle implementation and configuration identity.
    pub vehicle: ArtifactIdentity,
    /// The exact candidate-transition validator identity.
    pub transition_validator: ArtifactIdentity,
    /// The exact vehicle adjacency-policy identity.
    pub adjacency_policy_digest: Digest,
    /// The exact simulator-only direct-transport implementation identity.
    pub direct_transport: ArtifactIdentity,
    /// The exact runtime configuration identity.
    pub configuration: ArtifactIdentity,
}

/// The sealed production-input identity of one Aviate scenario runtime.
///
/// The value carries its own document, so a later attestation recomputes
/// the identity from the same bytes instead of trusting the sealed digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AviateRuntimeIdentity {
    document: RuntimeImplementationDocument,
    identity: ArtifactIdentity,
}

impl AviateRuntimeIdentity {
    /// Seals the complete production-input identity of this runtime.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the embedded inventory is not
    /// canonical, when its digest differs from the embedded digest, or when
    /// a bound identity is invalid.
    pub fn seal(inputs: &RuntimeIdentityInputs) -> Result<Self, AviateRuntimeError> {
        let sources = read_back_sources()?;
        for identity in [
            &inputs.vehicle,
            &inputs.transition_validator,
            &inputs.direct_transport,
            &inputs.configuration,
        ] {
            ArtifactIdentity::new(identity.id.clone(), identity.digest)
                .map_err(|source| AviateRuntimeError::InvalidIdentity { source })?;
        }
        if inputs.adjacency_policy_digest.is_zero() {
            return Err(AviateRuntimeError::IncompleteIdentity {
                detail: "the vehicle adjacency-policy digest is zero".to_owned(),
            });
        }
        let document = RuntimeImplementationDocument {
            schema_version: RUNTIME_IMPLEMENTATION_SCHEMA_VERSION,
            source_document_digest: Digest::from_bytes(RUNTIME_SOURCE_DIGEST),
            sources,
            vehicle: inputs.vehicle.clone(),
            transition_validator: inputs.transition_validator.clone(),
            adjacency_policy_digest: inputs.adjacency_policy_digest,
            direct_transport: inputs.direct_transport.clone(),
            configuration: inputs.configuration.clone(),
        };
        let identity = document.identity()?;
        Ok(Self { document, identity })
    }

    /// The canonical runtime implementation document.
    #[must_use]
    pub const fn document(&self) -> &RuntimeImplementationDocument {
        &self.document
    }

    /// The sealed runtime implementation identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    /// Recomputes this identity from the document it carries.
    ///
    /// Every external action runs this check first, so a runtime whose
    /// sealed digest no longer describes its own document cannot reach a
    /// journal, a process, a socket, a simulator, or a vehicle.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the recomputed identity differs
    /// from the sealed one.
    pub fn attest(&self) -> Result<(), AviateRuntimeError> {
        if self.document.identity()? == self.identity {
            return Ok(());
        }
        Err(AviateRuntimeError::RuntimeIdentityChanged)
    }

    /// Rejects a frozen session identity that this runtime does not match.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the runtime identity differs from
    /// the frozen one, or when this runtime no longer attests.
    pub fn require_frozen(&self, frozen: &ArtifactIdentity) -> Result<(), AviateRuntimeError> {
        self.attest()?;
        if &self.identity == frozen {
            return Ok(());
        }
        Err(AviateRuntimeError::RuntimeIdentityChanged)
    }
}

impl RuntimeImplementationDocument {
    /// Recomputes the identity of this canonical document.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the document is not canonical or
    /// cannot be encoded.
    pub fn identity(&self) -> Result<ArtifactIdentity, AviateRuntimeError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|source| AviateRuntimeError::Encode {
            document: "runtime implementation",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(IMPLEMENTATION_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        ArtifactIdentity::new(
            RUNTIME_IMPLEMENTATION_ID,
            Digest::from_bytes(hasher.finalize().into()),
        )
        .map_err(|source| AviateRuntimeError::InvalidIdentity { source })
    }

    fn validate(&self) -> Result<(), AviateRuntimeError> {
        if self.schema_version != RUNTIME_IMPLEMENTATION_SCHEMA_VERSION {
            return Err(AviateRuntimeError::IncompleteIdentity {
                detail: "the runtime implementation document has another schema".to_owned(),
            });
        }
        if self.source_document_digest.is_zero() || self.adjacency_policy_digest.is_zero() {
            return Err(AviateRuntimeError::IncompleteIdentity {
                detail: "the runtime implementation document has a zero digest".to_owned(),
            });
        }
        validate_entries(&self.sources)
    }
}

/// Reads the embedded inventory back and recomputes its digest.
///
/// The build script writes the document and its digest as two independent
/// constants. Recomputing one from the other is what makes a changed
/// production source a changed identity instead of a stale constant.
fn read_back_sources() -> Result<Vec<RuntimeSourceEntry>, AviateRuntimeError> {
    let bytes = RUNTIME_SOURCE_DOCUMENT.as_bytes();
    let parsed: RuntimeSourceDocument =
        serde_json::from_slice(bytes).map_err(|source| AviateRuntimeError::Decode {
            document: "runtime source inventory",
            source,
        })?;
    if parsed.schema_version != SOURCE_DOCUMENT_SCHEMA_VERSION {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: "the runtime source inventory has another schema".to_owned(),
        });
    }
    validate_entries(&parsed.entries)?;
    let canonical = serde_json::to_vec(&parsed).map_err(|source| AviateRuntimeError::Encode {
        document: "runtime source inventory",
        source,
    })?;
    if canonical != bytes {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: "the runtime source inventory is not canonical".to_owned(),
        });
    }
    if source_document_digest(&canonical) != RUNTIME_SOURCE_DIGEST {
        return Err(AviateRuntimeError::RuntimeIdentityChanged);
    }
    Ok(parsed.entries)
}

/// The digest the build script computes over the canonical inventory.
pub(crate) fn source_document_digest(document: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_DOCUMENT_DOMAIN);
    hasher.update((document.len() as u64).to_le_bytes());
    hasher.update(document);
    hasher.finalize().into()
}

/// Rejects an inventory that repeats, reorders, or names a test source.
fn validate_entries(entries: &[RuntimeSourceEntry]) -> Result<(), AviateRuntimeError> {
    if entries.is_empty() {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: "the runtime source inventory is empty".to_owned(),
        });
    }
    let names = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    if names.len() != entries.len() {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: "the runtime source inventory repeats a path".to_owned(),
        });
    }
    if entries.windows(2).any(|pair| pair[0].path >= pair[1].path) {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: "the runtime source inventory is not in canonical path order".to_owned(),
        });
    }
    for entry in entries {
        validate_entry(entry)?;
    }
    Ok(())
}

fn validate_entry(entry: &RuntimeSourceEntry) -> Result<(), AviateRuntimeError> {
    if is_test_path(&entry.path) {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: format!(
                "a test source entered the production identity: {}",
                entry.path
            ),
        });
    }
    if !entry.path.starts_with("src/")
        || !entry.path.ends_with(".rs")
        || entry.path.contains("..")
        || entry.path.contains('\\')
    {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: format!("an inventory path is not an owned source: {}", entry.path),
        });
    }
    let hex_is_valid = entry.sha256.len() == 32 * 2
        && entry
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !hex_is_valid || entry.bytes == 0 {
        return Err(AviateRuntimeError::IncompleteIdentity {
            detail: format!("an inventory entry has no content identity: {}", entry.path),
        });
    }
    Ok(())
}

/// Whether one inventory path names a test source.
///
/// The build script excludes the same names when it discovers inputs. This
/// check is the readback half: a document that reached the binary with a
/// test path in it cannot become a production identity.
pub(crate) fn is_test_path(path: &str) -> bool {
    path.split('/').any(|part| part == "tests")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs")
        || path.ends_with("/test_support.rs")
}
