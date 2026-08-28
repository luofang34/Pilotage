use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest as _, Sha256};

const DOCUMENT_SCHEMA_VERSION: u16 = 2;

#[derive(Debug)]
pub struct SourceIdentity {
    pub digest: [u8; 32],
    pub paths: Vec<PathBuf>,
}

#[derive(Serialize)]
struct SourceDocument {
    schema_version: u16,
    entries: Vec<SourceEntry>,
}

#[derive(Serialize)]
struct SourceEntry {
    path: String,
    sha256: String,
    bytes: u64,
}

pub fn calculate(workspace: &Path) -> Result<SourceIdentity, std::io::Error> {
    let mut paths = vec![
        workspace.join("crates/pilotage-mission-core/Cargo.toml"),
        workspace.join("tools/flight-tune/src/scenario_runtime.rs"),
    ];
    collect_production_rust(
        &workspace.join("crates/pilotage-mission-core/src"),
        &mut paths,
    )?;
    collect_production_rust(
        &workspace.join("tools/flight-tune/src/scenario_runtime"),
        &mut paths,
    )?;
    paths.sort();
    paths.dedup();
    let named = paths
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(workspace)
                .map_err(std::io::Error::other)?;
            Ok((portable(relative), std::fs::read(path)?))
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    Ok(SourceIdentity {
        digest: digest_named(&named)?,
        paths,
    })
}

fn collect_production_rust(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            collect_production_rust(&path, paths)?;
        } else if is_production_rust(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

fn is_production_rust(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.extension().is_some_and(|extension| extension == "rs")
        && name != "tests.rs"
        && name != "test_support.rs"
        && !name.ends_with("_tests.rs")
}

fn digest_named(inputs: &[(String, Vec<u8>)]) -> Result<[u8; 32], std::io::Error> {
    let mut ordered = inputs.to_vec();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let names = ordered
        .iter()
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>();
    if names.len() != ordered.len() {
        return Err(std::io::Error::other(
            "the scenario-runtime source inventory repeats a path",
        ));
    }
    let document = SourceDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        entries: ordered
            .into_iter()
            .filter(|(path, _)| test_source(path).is_none())
            .map(|(path, bytes)| SourceEntry {
                path,
                sha256: hex(&Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&document).map_err(std::io::Error::other)?;
    let mut hasher = Sha256::new();
    hasher.update(b"pilotage-scenario-engine-source-v2\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(hasher.finalize().into())
}

fn test_source(path: &str) -> Option<()> {
    let path = Path::new(path);
    if path.components().any(|part| part.as_os_str() == "tests")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name == "tests.rs" || name == "test_support.rs" || name.ends_with("_tests.rs")
            })
    {
        Some(())
    } else {
        None
    }
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
pub fn digest_named_for_test(inputs: &[(&str, &[u8])]) -> Result<[u8; 32], std::io::Error> {
    digest_named(
        &inputs
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), (*bytes).to_vec()))
            .collect::<Vec<_>>(),
    )
}
