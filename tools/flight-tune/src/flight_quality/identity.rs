//! The complete production-input identity of the flight-quality evaluators.
//!
//! The build script inventories every production source of the metric
//! evaluator and of the hard-gate evaluator, writes one canonical
//! schema-versioned document for each, and embeds each document together with
//! its digest. This module reads a document back, recomputes the digest from
//! the embedded bytes, and binds the result to the exact evaluator
//! configuration.
//!
//! Two values come out: the metric implementation identity and the hard-gate
//! implementation identity. A production source that changes changes the
//! identity of each evaluator that declares it. A test source cannot enter an
//! identity, because the inventory refuses a test path. Order cannot change an
//! identity, because the canonical document sorts its entries and the digest
//! frames each length. One inventory cannot stand in for the other, because
//! each document names its evaluator and each digest uses its own domain.
//!
//! SIM / NOT FOR FLIGHT.

use std::collections::BTreeSet;

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{ArtifactIdentity, EvaluatorError};

include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));

/// The stable name of the metric implementation identity.
pub const METRIC_IMPLEMENTATION_ID: &str = "pilotage-flight-quality-streaming-metrics-v2";

/// The stable name of the hard-gate implementation identity.
pub const GATE_IMPLEMENTATION_ID: &str = "pilotage-flight-quality-streaming-gates-v2";

/// The supported evaluator implementation document schema.
pub const EVALUATOR_IMPLEMENTATION_SCHEMA_VERSION: u16 = 1;

/// The schema version of the embedded source-inventory documents.
const SOURCE_DOCUMENT_SCHEMA_VERSION: u16 = 1;

/// The domain the build script separates the metric inventory with.
///
/// The value has to match the build script byte for byte. A readback that
/// recomputes the digest under another domain would accept a document the
/// build never wrote.
const METRIC_SOURCE_DOMAIN: &[u8] = b"flight-tune-metric-source-document-v1\0";

/// The domain the build script separates the hard-gate inventory with.
const GATE_SOURCE_DOMAIN: &[u8] = b"flight-tune-gate-source-document-v1\0";

/// The domain that separates one metric implementation document.
const METRIC_IMPLEMENTATION_DOMAIN: &[u8] = b"pilotage-flight-quality-streaming-metrics-v2\0";

/// The domain that separates one hard-gate implementation document.
const GATE_IMPLEMENTATION_DOMAIN: &[u8] = b"pilotage-flight-quality-streaming-gates-v2\0";

/// The repository roots that can hold one evaluator production source.
const OWNED_ROOTS: [&str; 5] = [
    "crates/pilotage-flight-quality/src/",
    "tools/flight-tune/build.rs",
    "tools/flight-tune/build_support/evaluator_source_identity.rs",
    "tools/flight-tune/src/flight_quality.rs",
    "tools/flight-tune/src/flight_quality/",
];

/// One flight-quality evaluator class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvaluatorClass {
    /// The continuous metric evaluator.
    Metric,
    /// The streaming hard-gate evaluator.
    Gate,
}

/// One production evaluator source and its exact content identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EvaluatorSourceEntry {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EvaluatorSourceDocument {
    schema_version: u16,
    evaluator: String,
    entries: Vec<EvaluatorSourceEntry>,
}

/// The canonical document that one evaluator identity is the digest of.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EvaluatorImplementationDocument {
    schema_version: u16,
    evaluator: String,
    source_document_digest: Digest,
    sources: Vec<EvaluatorSourceEntry>,
    configuration: Digest,
}

/// Binds one evaluator implementation to its exact configuration document.
pub(super) fn evaluator_identity<T: Serialize>(
    class: EvaluatorClass,
    config: &T,
) -> Result<ArtifactIdentity, EvaluatorError> {
    let sources = read_back_sources(class)?;
    let document = EvaluatorImplementationDocument {
        schema_version: EVALUATOR_IMPLEMENTATION_SCHEMA_VERSION,
        evaluator: label(class).to_owned(),
        source_document_digest: Digest::from_bytes(embedded_digest(class)),
        sources,
        configuration: configuration_digest(config)?,
    };
    document.identity(class)
}

impl EvaluatorImplementationDocument {
    fn identity(&self, class: EvaluatorClass) -> Result<ArtifactIdentity, EvaluatorError> {
        self.validate(class)?;
        let bytes = serde_json::to_vec(self).map_err(|error| {
            invalid(format!(
                "cannot encode the evaluator implementation document: {error}"
            ))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(implementation_domain(class));
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        ArtifactIdentity::new(
            implementation_id(class),
            Digest::from_bytes(hasher.finalize().into()),
        )
        .map_err(|error| invalid(error.to_string()))
    }

    fn validate(&self, class: EvaluatorClass) -> Result<(), EvaluatorError> {
        if self.schema_version != EVALUATOR_IMPLEMENTATION_SCHEMA_VERSION
            || self.evaluator != label(class)
        {
            return Err(invalid(
                "the evaluator implementation document has another schema",
            ));
        }
        if self.source_document_digest.is_zero() || self.configuration.is_zero() {
            return Err(invalid(
                "the evaluator implementation document has a zero digest",
            ));
        }
        validate_entries(&self.sources)
    }
}

/// Reads one embedded inventory back and recomputes its digest.
///
/// The build script writes the document and its digest as two independent
/// constants. Recomputing one from the other is what makes a changed
/// production source a changed identity instead of a stale constant.
fn read_back_sources(class: EvaluatorClass) -> Result<Vec<EvaluatorSourceEntry>, EvaluatorError> {
    let bytes = embedded_document(class).as_bytes();
    let parsed: EvaluatorSourceDocument = serde_json::from_slice(bytes).map_err(|error| {
        invalid(format!(
            "cannot read the evaluator source inventory back: {error}"
        ))
    })?;
    if parsed.schema_version != SOURCE_DOCUMENT_SCHEMA_VERSION || parsed.evaluator != label(class) {
        return Err(invalid("the evaluator source inventory has another schema"));
    }
    validate_entries(&parsed.entries)?;
    let canonical = serde_json::to_vec(&parsed).map_err(|error| {
        invalid(format!(
            "cannot encode the evaluator source inventory: {error}"
        ))
    })?;
    if canonical != bytes {
        return Err(invalid("the evaluator source inventory is not canonical"));
    }
    if source_document_digest(class, &parsed)? != embedded_digest(class) {
        return Err(invalid(
            "the evaluator source inventory does not match its embedded digest",
        ));
    }
    Ok(parsed.entries)
}

/// The digest the build script computes over one canonical inventory.
///
/// The build script frames each field rather than the encoded document. A
/// readback that hashed the JSON bytes instead would refuse every inventory
/// the build wrote.
fn source_document_digest(
    class: EvaluatorClass,
    document: &EvaluatorSourceDocument,
) -> Result<[u8; 32], EvaluatorError> {
    let mut hasher = Sha256::new();
    hasher.update(source_domain(class));
    append_frame(&mut hasher, &document.schema_version.to_le_bytes());
    append_frame(&mut hasher, document.evaluator.as_bytes());
    for entry in &document.entries {
        append_frame(&mut hasher, entry.path.as_bytes());
        append_frame(&mut hasher, &decode_sha256(&entry.sha256)?);
        append_frame(&mut hasher, &entry.bytes.to_le_bytes());
    }
    Ok(hasher.finalize().into())
}

fn append_frame(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn decode_sha256(value: &str) -> Result<[u8; 32], EvaluatorError> {
    let mut bytes = [0_u8; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        let pair = value
            .get(start..start + 2)
            .ok_or_else(|| invalid("an inventory entry has a short content identity"))?;
        *output = u8::from_str_radix(pair, 16)
            .map_err(|error| invalid(format!("an inventory entry is not hexadecimal: {error}")))?;
    }
    Ok(bytes)
}

fn configuration_digest<T: Serialize>(config: &T) -> Result<Digest, EvaluatorError> {
    let bytes = serde_json::to_vec(config)
        .map_err(|error| invalid(format!("cannot encode evaluator configuration: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

/// Rejects an inventory that repeats, reorders, or names a test source.
fn validate_entries(entries: &[EvaluatorSourceEntry]) -> Result<(), EvaluatorError> {
    if entries.is_empty() {
        return Err(invalid("the evaluator source inventory is empty"));
    }
    let names = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    if names.len() != entries.len() {
        return Err(invalid("the evaluator source inventory repeats a path"));
    }
    if entries.windows(2).any(|pair| pair[0].path >= pair[1].path) {
        return Err(invalid(
            "the evaluator source inventory is not in canonical path order",
        ));
    }
    for entry in entries {
        validate_entry(entry)?;
    }
    Ok(())
}

fn validate_entry(entry: &EvaluatorSourceEntry) -> Result<(), EvaluatorError> {
    if is_test_path(&entry.path) {
        return Err(invalid(format!(
            "a test source entered the production identity: {}",
            entry.path
        )));
    }
    if !is_owned_path(&entry.path) {
        return Err(invalid(format!(
            "an inventory path is not an owned source: {}",
            entry.path
        )));
    }
    let hex_is_valid = entry.sha256.len() == 32 * 2
        && entry
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !hex_is_valid || entry.bytes == 0 {
        return Err(invalid(format!(
            "an inventory entry has no content identity: {}",
            entry.path
        )));
    }
    Ok(())
}

/// Whether one inventory path stays inside the owned evaluator roots.
fn is_owned_path(path: &str) -> bool {
    path.ends_with(".rs")
        && !path.contains("..")
        && !path.contains('\\')
        && OWNED_ROOTS.iter().any(|root| path.starts_with(root))
}

/// Whether one inventory path names a test source.
///
/// The build script excludes the same names when it discovers inputs. This
/// check is the readback half: a document that reached the binary with a test
/// path in it cannot become a production identity.
fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/').any(|part| part == "tests")
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("test_")
}

const fn label(class: EvaluatorClass) -> &'static str {
    match class {
        EvaluatorClass::Metric => "metric",
        EvaluatorClass::Gate => "hard_gates",
    }
}

const fn implementation_id(class: EvaluatorClass) -> &'static str {
    match class {
        EvaluatorClass::Metric => METRIC_IMPLEMENTATION_ID,
        EvaluatorClass::Gate => GATE_IMPLEMENTATION_ID,
    }
}

const fn source_domain(class: EvaluatorClass) -> &'static [u8] {
    match class {
        EvaluatorClass::Metric => METRIC_SOURCE_DOMAIN,
        EvaluatorClass::Gate => GATE_SOURCE_DOMAIN,
    }
}

const fn implementation_domain(class: EvaluatorClass) -> &'static [u8] {
    match class {
        EvaluatorClass::Metric => METRIC_IMPLEMENTATION_DOMAIN,
        EvaluatorClass::Gate => GATE_IMPLEMENTATION_DOMAIN,
    }
}

const fn embedded_document(class: EvaluatorClass) -> &'static str {
    match class {
        EvaluatorClass::Metric => METRIC_SOURCE_DOCUMENT,
        EvaluatorClass::Gate => GATE_SOURCE_DOCUMENT,
    }
}

const fn embedded_digest(class: EvaluatorClass) -> [u8; 32] {
    match class {
        EvaluatorClass::Metric => METRIC_SOURCE_DIGEST,
        EvaluatorClass::Gate => GATE_SOURCE_DIGEST,
    }
}

fn invalid(detail: impl Into<String>) -> EvaluatorError {
    EvaluatorError::new(detail)
}

#[cfg(test)]
#[path = "identity/tests.rs"]
mod tests;
