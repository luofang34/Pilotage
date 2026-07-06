//! Generates `prost` wire types from
//! `schemas/pilotage/bridge/v1/bridge.proto` into `OUT_DIR` at build time,
//! mirroring `pilotage-protocol`'s build script. This is the internal
//! host<->sidecar bridge schema (ADR-0008), never the public client
//! protocol.
//!
//! `println!` is the only channel Cargo defines for a build script to emit
//! `cargo:rerun-if-changed` and `cargo:warning` directives, so it is allowed
//! here even though the workspace otherwise bans it in favor of `tracing`.
#![allow(clippy::disallowed_macros)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bridge_dir = manifest_dir
        .join("..")
        .join("..")
        .join("schemas")
        .join("pilotage")
        .join("bridge")
        .join("v1");
    let schema_root = manifest_dir.join("..").join("..").join("schemas");

    let protos = [bridge_dir.join("bridge.proto")];

    println!("cargo:rerun-if-changed={}", bridge_dir.display());

    // A build script cannot propagate `Result` to `main`'s caller the way a
    // library can (ADR-0015 bans `process::exit`), so a non-zero `ExitCode`
    // is the sanctioned way to fail the build without `expect`/`panic`.
    match prost_build::Config::new().compile_protos(&protos, &[schema_root]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            println!("cargo:warning=failed to compile pilotage.bridge.v1 schemas: {err}");
            ExitCode::FAILURE
        }
    }
}
