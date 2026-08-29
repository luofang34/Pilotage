//! Creates a compile-time identity for the tuning harness sources.
//!
//! `println!` is the Cargo build-script protocol channel. It is not a
//! diagnostic channel in this file.

#![allow(clippy::disallowed_macros)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest as _, Sha256};

#[path = "build_support/evaluator_source_identity.rs"]
mod evaluator_source_identity;
#[path = "build_support/scenario_runtime_identity.rs"]
mod scenario_runtime_identity;

fn main() -> ExitCode {
    match generate_build_inputs() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            println!("cargo:warning={error}");
            ExitCode::FAILURE
        }
    }
}

fn generate_build_inputs() -> Result<(), std::io::Error> {
    let workspace = workspace_root();
    generate_evaluator_identities(&workspace)
        .map_err(|error| labelled("cannot identify flight-quality evaluator sources", &error))?;
    generate_scenario_runtime_identity(&workspace)
        .map_err(|error| labelled("cannot identify scenario runtime sources", &error))?;
    let identity = source_identity()
        .map_err(|error| labelled("cannot identify flight-tune sources", &error))?;
    println!("cargo:rustc-env=FLIGHT_TUNE_BUILD_ID={identity}");
    Ok(())
}

fn labelled(detail: &str, error: &std::io::Error) -> std::io::Error {
    std::io::Error::other(format!("{detail}: {error}"))
}

fn generate_scenario_runtime_identity(workspace: &Path) -> Result<(), std::io::Error> {
    let runtime = scenario_runtime_identity::calculate(workspace)?;
    println!(
        "cargo:rustc-env=FLIGHT_TUNE_SCENARIO_ENGINE_ID={}",
        hex(&runtime.digest)
    );
    for path in runtime.paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    Ok(())
}

/// Writes the closed evaluator inventories the crate reads back at run time.
///
/// Both inventories are calculated before either is written. A partial file
/// would let a build that failed completeness still compile against a stale
/// constant from an earlier run.
fn generate_evaluator_identities(workspace: &Path) -> Result<(), std::io::Error> {
    let metric = evaluator_source_identity::calculate(
        workspace,
        evaluator_source_identity::EvaluatorKind::Metric,
    )?;
    let gates = evaluator_source_identity::calculate(
        workspace,
        evaluator_source_identity::EvaluatorKind::Gate,
    )?;
    for root in evaluator_source_identity::source_roots(workspace) {
        println!("cargo:rerun-if-changed={}", root.display());
    }
    for path in metric.paths.iter().chain(&gates.paths) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let output = output_directory()?.join("evaluator_source_identity.rs");
    let mut file = std::fs::File::create(output)?;
    write_inventory(&mut file, "METRIC", metric)?;
    write_inventory(&mut file, "GATE", gates)?;
    file.sync_all()
}

fn output_directory() -> Result<PathBuf, std::io::Error> {
    std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("Cargo did not supply the build output directory"))
}

fn write_inventory(
    file: &mut std::fs::File,
    name: &str,
    inventory: evaluator_source_identity::EvaluatorSourceInventory,
) -> Result<(), std::io::Error> {
    if inventory.names.len() != inventory.paths.len() {
        return Err(std::io::Error::other(
            "an evaluator source inventory is inconsistent",
        ));
    }
    let document = String::from_utf8(inventory.document).map_err(std::io::Error::other)?;
    writeln!(file, "const {name}_SOURCE_DOCUMENT: &str = {document:?};")?;
    writeln!(
        file,
        "const {name}_SOURCE_DIGEST: [u8; 32] = {:?};",
        inventory.digest
    )?;
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn source_identity() -> Result<String, std::io::Error> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.join("../..");
    let roots = [
        ("flight-tune", manifest.join("src")),
        (
            "pilotage-flight-quality",
            workspace.join("crates/pilotage-flight-quality/src"),
        ),
        (
            "pilotage-trial",
            workspace.join("crates/pilotage-trial/src"),
        ),
    ];
    let mut hasher = Sha256::new();
    hash_file(
        &mut hasher,
        "flight-tune/Cargo.toml",
        &manifest.join("Cargo.toml"),
    )?;
    hash_file(
        &mut hasher,
        "flight-tune/build.rs",
        &manifest.join("build.rs"),
    )?;
    hash_file(
        &mut hasher,
        "workspace/Cargo.toml",
        &workspace.join("Cargo.toml"),
    )?;
    hash_file(
        &mut hasher,
        "workspace/Cargo.lock",
        &workspace.join("Cargo.lock"),
    )?;
    for (label, root) in roots {
        hash_tree(&mut hasher, label, &root)?;
    }
    for name in ["TARGET", "PROFILE", "OPT_LEVEL", "DEBUG"] {
        hasher.update(name.as_bytes());
        hasher.update(std::env::var(name).unwrap_or_default().as_bytes());
    }
    Ok(hex(&hasher.finalize()))
}

fn hash_tree(hasher: &mut Sha256, label: &str, root: &Path) -> Result<(), std::io::Error> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    files.sort();
    for path in files {
        let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
        let name = format!("{label}/{}", relative.to_string_lossy());
        hash_file(hasher, &name, &path)?;
    }
    Ok(())
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn hash_file(hasher: &mut Sha256, name: &str, path: &Path) -> Result<(), std::io::Error> {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    hasher.update((name.len() as u64).to_le_bytes());
    hasher.update(name.as_bytes());
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
