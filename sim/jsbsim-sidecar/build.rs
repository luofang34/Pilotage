//! With the `cpp` feature, compiles the C++ shim over JSBSim and links
//! JSBSim's static library. Without it there is nothing to build.
//!
//! JSBSim is an external checkout: `JSBSIM_DIR`, else `~/jsbsim`, with
//! its build installed in `install/` by `scripts/build-jsbsim.sh` (which
//! `cargo xtask build-jsbsim` runs). The
//! checkout's path is also handed to the crate as `PILOTAGE_JSBSIM_ROOT`,
//! the default root for JSBSim's engine and systems data.
//!
//! `println!` is the only channel Cargo defines for a build script to emit
//! its directives, so it is allowed here even though the workspace
//! otherwise bans it in favor of `tracing`.
#![allow(clippy::disallowed_macros)]

use std::process::ExitCode;

#[cfg(not(feature = "cpp"))]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}

#[cfg(feature = "cpp")]
fn jsbsim_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    match std::env::var_os("JSBSIM_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join("jsbsim")),
    }
}

#[cfg(feature = "cpp")]
fn main() -> ExitCode {
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-env-changed=JSBSIM_DIR");
    println!("cargo:rerun-if-env-changed=HOME");
    let Some(root) = jsbsim_dir() else {
        println!("cargo:warning=set JSBSIM_DIR to the JSBSim checkout");
        return ExitCode::FAILURE;
    };
    let prefix = root.join("install");
    let library = prefix.join("lib/libJSBSim.a");
    let include = prefix.join("include/JSBSim");
    if !library.is_file() || !include.join("FGFDMExec.h").is_file() {
        println!(
            "cargo:warning=JSBSim is not built at {}: run scripts/build-jsbsim.sh \
             (or set JSBSIM_DIR)",
            prefix.display()
        );
        return ExitCode::FAILURE;
    }
    println!("cargo:rerun-if-changed={}", library.display());
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("shim/jsbsim_shim.cpp")
        .include(&include)
        // JSBSim's own headers warn under -Wall; the shim's code is small
        // and reviewed, so the flood would hide nothing useful.
        .warnings(false)
        .compile("pilotage_jsbsim_shim");
    // After the shim, so a single-pass linker resolves the shim's JSBSim
    // symbols from the library that follows it.
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=JSBSim");
    println!("cargo:rustc-env=PILOTAGE_JSBSIM_ROOT={}", root.display());
    ExitCode::SUCCESS
}
