//! Complete production-source inventories for flight-quality evaluators.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const DOCUMENT_SCHEMA_VERSION: u16 = 1;
const METRIC_DOMAIN: &[u8] = b"flight-tune-metric-source-document-v1\0";
const GATE_DOMAIN: &[u8] = b"flight-tune-gate-source-document-v1\0";

/// Metric production inputs in canonical repository-relative order.
pub const METRIC_PRODUCTION_INPUTS: [&str; 19] = [
    "crates/pilotage-flight-quality/src/angular.rs",
    "crates/pilotage-flight-quality/src/angular_release.rs",
    "crates/pilotage-flight-quality/src/collective.rs",
    "crates/pilotage-flight-quality/src/control.rs",
    "crates/pilotage-flight-quality/src/error.rs",
    "crates/pilotage-flight-quality/src/lib.rs",
    "crates/pilotage-flight-quality/src/release.rs",
    "crates/pilotage-flight-quality/src/response.rs",
    "crates/pilotage-flight-quality/src/sample.rs",
    "crates/pilotage-flight-quality/src/series.rs",
    "crates/pilotage-flight-quality/src/signal.rs",
    "crates/pilotage-flight-quality/src/vocabulary.rs",
    "tools/flight-tune/build.rs",
    "tools/flight-tune/build_support/evaluator_source_identity.rs",
    "tools/flight-tune/src/flight_quality.rs",
    "tools/flight-tune/src/flight_quality/config.rs",
    "tools/flight-tune/src/flight_quality/identity.rs",
    "tools/flight-tune/src/flight_quality/metrics.rs",
    "tools/flight-tune/src/flight_quality/telemetry.rs",
];

/// Hard-gate production inputs in canonical repository-relative order.
pub const GATE_PRODUCTION_INPUTS: [&str; 10] = [
    "crates/pilotage-flight-quality/src/error.rs",
    "crates/pilotage-flight-quality/src/gate.rs",
    "crates/pilotage-flight-quality/src/lib.rs",
    "tools/flight-tune/build.rs",
    "tools/flight-tune/build_support/evaluator_source_identity.rs",
    "tools/flight-tune/src/flight_quality.rs",
    "tools/flight-tune/src/flight_quality/config.rs",
    "tools/flight-tune/src/flight_quality/gates.rs",
    "tools/flight-tune/src/flight_quality/identity.rs",
    "tools/flight-tune/src/flight_quality/telemetry.rs",
];

/// One evaluator source class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluatorKind {
    /// Continuous flight-quality metrics.
    Metric,
    /// Fail-fast flight-quality gates.
    Gate,
}

/// One calculated evaluator source inventory.
pub struct EvaluatorSourceInventory {
    /// Content identity of the canonical inventory document.
    pub digest: [u8; 32],
    /// Canonical schema-versioned inventory document.
    pub document: Vec<u8>,
    /// Stable repository-relative production input names.
    pub names: Vec<String>,
    /// Exact input paths for Cargo rebuild tracking.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceDocument {
    schema_version: u16,
    evaluator: String,
    entries: Vec<SourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceEntry {
    path: String,
    sha256: String,
    bytes: u64,
}

/// Returns all roots that can add one evaluator production source.
pub fn source_roots(workspace: &Path) -> [PathBuf; 5] {
    [
        workspace.join("tools/flight-tune/build.rs"),
        workspace.join("tools/flight-tune/build_support/evaluator_source_identity.rs"),
        workspace.join("tools/flight-tune/src/flight_quality.rs"),
        workspace.join("tools/flight-tune/src/flight_quality"),
        workspace.join("crates/pilotage-flight-quality/src"),
    ]
}

/// Calculates one complete evaluator production identity.
pub fn calculate(
    workspace: &Path,
    kind: EvaluatorKind,
) -> Result<EvaluatorSourceInventory, std::io::Error> {
    validate_completeness(workspace)?;
    let inputs = read_declared_inputs(workspace, kind)?;
    let document = document_from_named_bytes(kind, &inputs)?;
    let digest = digest_document(kind, &document)?;
    let names = inputs.iter().map(|(name, _)| name.clone()).collect();
    let paths = inputs
        .iter()
        .map(|(name, _)| workspace.join(name))
        .collect();
    Ok(EvaluatorSourceInventory {
        digest,
        document,
        names,
        paths,
    })
}

/// Calculates an identity from named source bytes for inventory tests.
#[cfg(test)]
pub fn digest_named_bytes(
    kind: EvaluatorKind,
    inputs: &[(String, Vec<u8>)],
) -> Result<[u8; 32], std::io::Error> {
    let document = document_from_named_bytes(kind, inputs)?;
    digest_document(kind, &document)
}

/// Validates one generated document and recomputes its digest.
pub fn digest_document(kind: EvaluatorKind, bytes: &[u8]) -> Result<[u8; 32], std::io::Error> {
    let document = readback_document(kind, bytes)?;
    let mut hasher = Sha256::new();
    hasher.update(domain(kind));
    append_frame(&mut hasher, &document.schema_version.to_le_bytes());
    append_frame(&mut hasher, document.evaluator.as_bytes());
    for entry in document.entries {
        append_frame(&mut hasher, entry.path.as_bytes());
        append_frame(&mut hasher, &decode_sha256(&entry.sha256)?);
        append_frame(&mut hasher, &entry.bytes.to_le_bytes());
    }
    Ok(hasher.finalize().into())
}

fn validate_completeness(workspace: &Path) -> Result<(), std::io::Error> {
    let metric = declared_names(EvaluatorKind::Metric)?;
    let gates = declared_names(EvaluatorKind::Gate)?;
    let declared = metric.union(&gates).cloned().collect::<BTreeSet<_>>();
    let discovered = discover_production_inputs(workspace)?;
    if declared == discovered {
        return Ok(());
    }
    let missing = declared
        .difference(&discovered)
        .cloned()
        .collect::<Vec<_>>();
    let extra = discovered
        .difference(&declared)
        .cloned()
        .collect::<Vec<_>>();
    Err(std::io::Error::other(format!(
        "the evaluator source inventory differs: missing={missing:?}, extra={extra:?}"
    )))
}

fn declared_names(kind: EvaluatorKind) -> Result<BTreeSet<String>, std::io::Error> {
    let inputs = declared(kind);
    let names = inputs
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    if names.len() != inputs.len() {
        return Err(std::io::Error::other(
            "an evaluator inventory contains a repeated path",
        ));
    }
    if inputs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(std::io::Error::other(
            "an evaluator inventory is not in canonical path order",
        ));
    }
    Ok(names)
}

fn discover_production_inputs(workspace: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut discovered = Vec::new();
    for root in source_roots(workspace) {
        if root.is_dir() {
            collect_files(&root, workspace, &mut discovered)?;
        } else {
            inspect_file(&root, workspace, &mut discovered)?;
        }
    }
    let unique = discovered.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != discovered.len() {
        return Err(std::io::Error::other(
            "the evaluator source roots discover a repeated path",
        ));
    }
    Ok(unique)
}

fn collect_files(
    directory: &Path,
    workspace: &Path,
    discovered: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    reject_symlink(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        reject_symlink(&path)?;
        if path.is_dir() {
            if path.file_name().is_none_or(|name| name != "tests") {
                collect_files(&path, workspace, discovered)?;
            }
        } else if is_production_rust(&path) {
            inspect_file(&path, workspace, discovered)?;
        }
    }
    Ok(())
}

fn inspect_file(
    path: &Path,
    workspace: &Path,
    discovered: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    reject_symlink(path)?;
    validate_owned_file(workspace, path)?;
    let relative = path.strip_prefix(workspace).map_err(io_other)?;
    discovered.push(portable(relative));
    Ok(())
}

fn read_declared_inputs(
    workspace: &Path,
    kind: EvaluatorKind,
) -> Result<Vec<(String, Vec<u8>)>, std::io::Error> {
    declared(kind)
        .iter()
        .map(|name| {
            let path = workspace.join(name);
            validate_owned_file(workspace, &path)?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?.read_to_end(&mut bytes)?;
            Ok(((*name).to_owned(), bytes))
        })
        .collect()
}

fn validate_owned_file(workspace: &Path, path: &Path) -> Result<(), std::io::Error> {
    reject_symlink(path)?;
    let canonical_workspace = std::fs::canonicalize(workspace)?;
    let canonical = std::fs::canonicalize(path)?;
    if !canonical.starts_with(&canonical_workspace) || !canonical.is_file() {
        return Err(std::io::Error::other(
            "an evaluator source is outside the workspace root",
        ));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), std::io::Error> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        Err(std::io::Error::other(
            "the evaluator source inventory does not permit symlinks",
        ))
    } else {
        Ok(())
    }
}

fn document_from_named_bytes(
    kind: EvaluatorKind,
    inputs: &[(String, Vec<u8>)],
) -> Result<Vec<u8>, std::io::Error> {
    let expected = declared_names(kind)?;
    let mut ordered = inputs.to_vec();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let names = ordered
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    if names != expected || names.len() != ordered.len() {
        return Err(std::io::Error::other(
            "the evaluator byte inventory does not match its declared paths",
        ));
    }
    let document = SourceDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        evaluator: label(kind).to_owned(),
        entries: ordered
            .into_iter()
            .map(|(path, bytes)| SourceEntry {
                path,
                sha256: sha256_hex(&bytes),
                bytes: bytes.len() as u64,
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&document).map_err(io_other)?;
    readback_document(kind, &bytes).map(|_| bytes)
}

fn readback_document(kind: EvaluatorKind, bytes: &[u8]) -> Result<SourceDocument, std::io::Error> {
    let document: SourceDocument = serde_json::from_slice(bytes).map_err(io_other)?;
    let canonical = serde_json::to_vec(&document).map_err(io_other)?;
    let names = document
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    let valid = canonical == bytes
        && document.schema_version == DOCUMENT_SCHEMA_VERSION
        && document.evaluator == label(kind)
        && names == declared_names(kind)?
        && names.len() == document.entries.len()
        && document
            .entries
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        && document.entries.iter().all(|entry| {
            entry.bytes > 0
                && entry.sha256.len() == 64
                && entry
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if valid {
        Ok(document)
    } else {
        Err(std::io::Error::other(
            "the evaluator source document is not canonical and complete",
        ))
    }
}

fn declared(kind: EvaluatorKind) -> &'static [&'static str] {
    match kind {
        EvaluatorKind::Metric => &METRIC_PRODUCTION_INPUTS,
        EvaluatorKind::Gate => &GATE_PRODUCTION_INPUTS,
    }
}

fn domain(kind: EvaluatorKind) -> &'static [u8] {
    match kind {
        EvaluatorKind::Metric => METRIC_DOMAIN,
        EvaluatorKind::Gate => GATE_DOMAIN,
    }
}

fn label(kind: EvaluatorKind) -> &'static str {
    match kind {
        EvaluatorKind::Metric => "metric",
        EvaluatorKind::Gate => "hard_gates",
    }
}

fn append_frame(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn decode_sha256(value: &str) -> Result<[u8; 32], std::io::Error> {
    let mut bytes = [0_u8; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *output = u8::from_str_radix(&value[start..start + 2], 16).map_err(io_other)?;
    }
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_production_rust(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.extension().is_some_and(|extension| extension == "rs")
        && name != "tests.rs"
        && !name.ends_with("_tests.rs")
        && !name.starts_with("test_")
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn io_other(error: impl ToString) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
