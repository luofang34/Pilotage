//! Complete source inventory for the Aviate scenario runtime.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const DOCUMENT_SCHEMA_VERSION: u16 = 1;

/// Production runtime inputs that require an explicit identity binding.
pub const PRODUCTION_INPUTS: [&str; 16] = [
    "src/runtime.rs",
    "src/runtime/conditions.rs",
    "src/runtime/direct.rs",
    "src/runtime/identity.rs",
    "src/runtime/math.rs",
    "src/runtime/phase.rs",
    "src/runtime/phase/direct.rs",
    "src/runtime/phase/direct/ledger.rs",
    "src/runtime/phase/direct/readback.rs",
    "src/runtime/phase/transition.rs",
    "src/runtime/phase/waveform.rs",
    "src/runtime/preparation.rs",
    "src/runtime/quality.rs",
    "src/runtime/telemetry.rs",
    "src/runtime/terminal.rs",
    "src/runtime/timing.rs",
];

/// One calculated scenario-runtime source inventory.
pub struct RuntimeSourceInventory {
    /// Content identity of the canonical inventory document.
    pub digest: [u8; 32],
    /// Canonical schema-versioned inventory document.
    pub document: Vec<u8>,
    /// Stable package-relative production input names.
    pub names: Vec<String>,
    /// Exact input paths for Cargo rebuild tracking.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSourceDocument {
    schema_version: u16,
    entries: Vec<RuntimeSourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSourceEntry {
    path: String,
    sha256: String,
    bytes: u64,
}

/// Returns all roots that can add one production runtime source.
pub fn source_roots(manifest: &Path) -> Vec<PathBuf> {
    vec![
        manifest.join("src/runtime"),
        manifest.join("src/runtime.rs"),
    ]
}

/// Calculates the complete scenario-runtime production identity.
pub fn calculate(manifest: &Path) -> Result<RuntimeSourceInventory, std::io::Error> {
    validate_completeness(manifest)?;
    let inputs = read_declared_inputs(manifest)?;
    let document = document_from_named_bytes(&inputs)?;
    let digest = digest_document(&document);
    let names = inputs.iter().map(|(name, _)| name.clone()).collect();
    let paths = inputs.iter().map(|(name, _)| manifest.join(name)).collect();
    Ok(RuntimeSourceInventory {
        digest,
        document,
        names,
        paths,
    })
}

/// Calculates the identity of named production bytes in canonical path order.
#[cfg(test)]
pub fn digest_named_bytes(inputs: &[(String, Vec<u8>)]) -> Result<[u8; 32], std::io::Error> {
    document_from_named_bytes(inputs).map(|document| digest_document(&document))
}

/// Validates and returns the canonical source document entries.
pub fn readback_document(bytes: &[u8]) -> Result<Vec<(String, String, u64)>, std::io::Error> {
    let parsed: RuntimeSourceDocument = serde_json::from_slice(bytes).map_err(io_other)?;
    validate_document(&parsed)?;
    let canonical = serde_json::to_vec(&parsed).map_err(io_other)?;
    if canonical != bytes {
        return Err(std::io::Error::other(
            "the runtime-source document is not canonical",
        ));
    }
    Ok(parsed
        .entries
        .into_iter()
        .map(|entry| (entry.path, entry.sha256, entry.bytes))
        .collect())
}

fn validate_completeness(manifest: &Path) -> Result<(), std::io::Error> {
    let declared = declared_names()?;
    let discovered = discover_production_inputs(manifest)?;
    if declared != discovered {
        let missing = declared
            .difference(&discovered)
            .cloned()
            .collect::<Vec<_>>();
        let extra = discovered
            .difference(&declared)
            .cloned()
            .collect::<Vec<_>>();
        return Err(std::io::Error::other(format!(
            "the runtime-source inventory differs: missing={missing:?}, extra={extra:?}"
        )));
    }
    Ok(())
}

fn declared_names() -> Result<BTreeSet<String>, std::io::Error> {
    let declared = PRODUCTION_INPUTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    if declared.len() != PRODUCTION_INPUTS.len() {
        return Err(std::io::Error::other(
            "the declared runtime-source inventory contains a repeated path",
        ));
    }
    Ok(declared)
}

fn discover_production_inputs(manifest: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut discovered = Vec::new();
    inspect_file(&manifest.join("src/runtime.rs"), manifest, &mut discovered)?;
    collect_files(&manifest.join("src/runtime"), manifest, &mut discovered)?;
    Ok(discovered.into_iter().collect())
}

fn collect_files(
    directory: &Path,
    manifest: &Path,
    discovered: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    reject_symlink(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        reject_symlink(&path)?;
        if path.is_dir() {
            if path.file_name().is_none_or(|name| name != "tests") {
                collect_files(&path, manifest, discovered)?;
            }
        } else if is_production_rust(&path) {
            inspect_file(&path, manifest, discovered)?;
        }
    }
    Ok(())
}

fn inspect_file(
    path: &Path,
    manifest: &Path,
    discovered: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    reject_symlink(path)?;
    if !path.is_file() {
        return Err(std::io::Error::other("a runtime source is not a file"));
    }
    let relative = path.strip_prefix(manifest).map_err(io_other)?;
    discovered.push(portable(relative));
    Ok(())
}

fn read_declared_inputs(manifest: &Path) -> Result<Vec<(String, Vec<u8>)>, std::io::Error> {
    PRODUCTION_INPUTS
        .iter()
        .map(|name| {
            let path = manifest.join(name);
            validate_owned_file(manifest, &path)?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?.read_to_end(&mut bytes)?;
            Ok(((*name).to_owned(), bytes))
        })
        .collect()
}

fn validate_owned_file(manifest: &Path, path: &Path) -> Result<(), std::io::Error> {
    reject_symlink(path)?;
    let canonical_manifest = std::fs::canonicalize(manifest)?;
    let canonical = std::fs::canonicalize(path)?;
    if !canonical.starts_with(&canonical_manifest) || !canonical.is_file() {
        return Err(std::io::Error::other(
            "a runtime source is outside the package root",
        ));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), std::io::Error> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        Err(std::io::Error::other(
            "the runtime-source inventory does not permit symlinks",
        ))
    } else {
        Ok(())
    }
}

fn document_from_named_bytes(inputs: &[(String, Vec<u8>)]) -> Result<Vec<u8>, std::io::Error> {
    let mut ordered = inputs.to_vec();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    if ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(std::io::Error::other(
            "the runtime-source byte inventory contains a repeated path",
        ));
    }
    let document = RuntimeSourceDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        entries: ordered
            .into_iter()
            .map(|(path, bytes)| RuntimeSourceEntry {
                path,
                sha256: sha256_hex(&bytes),
                bytes: bytes.len() as u64,
            })
            .collect(),
    };
    validate_document(&document)?;
    let bytes = serde_json::to_vec(&document).map_err(io_other)?;
    readback_document(&bytes).map(|_| bytes)
}

fn validate_document(document: &RuntimeSourceDocument) -> Result<(), std::io::Error> {
    let paths = document
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    let valid = document.schema_version == DOCUMENT_SCHEMA_VERSION
        && paths == declared_names()?
        && document.entries.len() == PRODUCTION_INPUTS.len()
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
        Ok(())
    } else {
        Err(std::io::Error::other(
            "the runtime-source document is invalid",
        ))
    }
}

fn digest_document(document: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"flight-tune-aviate-runtime-source-document-v1\0");
    hasher.update((document.len() as u64).to_le_bytes());
    hasher.update(document);
    hasher.finalize().into()
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
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn io_other(error: impl ToString) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
