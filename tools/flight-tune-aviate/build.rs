//! Creates the complete Aviate scenario-runtime source identity.
//!
//! `println!` is the Cargo build-script protocol channel. It is not a
//! diagnostic channel in this file.

#![allow(clippy::disallowed_macros)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[path = "build_support/runtime_source_identity.rs"]
mod runtime_source_identity;

fn main() -> ExitCode {
    match generate_identity() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            println!("cargo:warning=cannot identify the Aviate runtime sources: {error}");
            ExitCode::FAILURE
        }
    }
}

fn generate_identity() -> Result<(), std::io::Error> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inventory = runtime_source_identity::calculate(&manifest)?;
    if inventory.names.len() != inventory.paths.len() {
        return Err(std::io::Error::other(
            "the runtime-source inventory is inconsistent",
        ));
    }
    for root in runtime_source_identity::source_roots(&manifest) {
        println!("cargo:rerun-if-changed={}", root.display());
    }
    for path in inventory.paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let output =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or_else(|| {
            std::io::Error::other("Cargo did not supply the build output directory")
        })?)
        .join("runtime_source_identity.rs");
    let mut file = std::fs::File::create(output)?;
    let document = String::from_utf8(inventory.document).map_err(std::io::Error::other)?;
    writeln!(
        file,
        "const RUNTIME_SOURCE_DOCUMENT: &str = {:?};",
        document
    )?;
    writeln!(
        file,
        "const RUNTIME_SOURCE_DIGEST: [u8; 32] = {:?};",
        inventory.digest
    )?;
    file.sync_all()
}
