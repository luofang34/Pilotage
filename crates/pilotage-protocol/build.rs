//! Generates `prost` wire types from `schemas/pilotage/v1/*.proto` into
//! `OUT_DIR` at build time (ADR-0014). Nothing under `schemas/` is committed
//! as generated Rust; `src/wire.rs` includes the generated file at compile
//! time via `include!`.
//!
//! `println!` is the only channel Cargo defines for a build script to emit
//! `cargo:rerun-if-changed` and `cargo:warning` directives, so it is allowed
//! here even though the workspace otherwise bans it in favor of `tracing`.
#![allow(clippy::disallowed_macros)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schemas_dir = manifest_dir
        .join("..")
        .join("..")
        .join("schemas")
        .join("pilotage")
        .join("v1");
    let schema_root = manifest_dir.join("..").join("..").join("schemas");

    let protos = [
        "common.proto",
        "control.proto",
        "authority.proto",
        "telemetry.proto",
        "capability.proto",
        "session.proto",
        "envelope.proto",
    ]
    .map(|name| schemas_dir.join(name));

    println!("cargo:rerun-if-changed={}", schemas_dir.display());

    // A build script cannot propagate `Result` to `main`'s caller the way a
    // library can (ADR-0015 bans `process::exit`), so a non-zero `ExitCode`
    // is the sanctioned way to fail the build without `expect`/`panic`.
    // The truth/FC-state messages ride inside TelemetrySample, whose
    // envelope Payload variant must stay comparable in size to its
    // siblings; boxing keeps the rarely-populated oracle lanes off the
    // hot variant.
    match prost_build::Config::new()
        .boxed(".pilotage.v1.TelemetrySample.sim_truth")
        .boxed(".pilotage.v1.TelemetrySample.fc_state")
        .compile_protos(&protos, &[schema_root])
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            println!("cargo:warning=failed to compile pilotage.v1 schemas: {err}");
            ExitCode::FAILURE
        }
    }
}
