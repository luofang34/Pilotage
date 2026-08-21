//! Creates a compile-time identity for the tuning harness sources.
//!
//! `println!` is the Cargo build-script protocol channel. It is not a
//! diagnostic channel in this file.

#![allow(clippy::disallowed_macros)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest as _, Sha256};

fn main() -> ExitCode {
    match source_identity() {
        Ok(identity) => {
            println!("cargo:rustc-env=FLIGHT_TUNE_BUILD_ID={identity}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("cargo:warning=cannot identify flight-tune sources: {error}");
            ExitCode::FAILURE
        }
    }
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
