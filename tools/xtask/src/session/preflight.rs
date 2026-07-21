//! Out-of-the-box build preflight: turns a fresh checkout into a runnable
//! session by producing the artifacts git does not track — the release host
//! binary and the viewer's generated wasm runtime — before any stage starts.
//! Backend-specific gitignored artifacts (the px4 camera sidecar) are each
//! backend's own best-effort `prepare`.

use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

pub(crate) mod stamp;

/// Where preflight stamps live (under the gitignored build tree).
const WEB_RUNTIME_STAMP: &str = "target/xtask-stamps/web-runtime.stamp";

/// The source inputs whose working-tree content decides whether the viewer
/// wasm runtime is stale: the two wasm crates, their engine dependency, and
/// the build script itself.
pub(crate) const WEB_RUNTIME_SOURCES: [&str; 5] = [
    "clients/web-control",
    "clients/web-instruments",
    "crates/pilotage-input",
    "crates/pilotage-instrument-panels",
    "scripts/build-web-instruments.sh",
];

/// Builds the session host in release, the binary every session runs.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when the build cannot spawn or fails.
pub(super) fn build_host(repo_root: &std::path::Path) -> Result<(), XtaskError> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "pilotage-session-host"])
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "building the session host",
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::CommandFailed {
            context: "cargo build --release -p pilotage-session-host",
            status: status.to_string(),
        })
    }
}

/// The viewer's generated wasm runtime files (all gitignored). The viewer's
/// `main.js` statically imports `instrument-runtime.js`, so a checkout missing
/// these serves a viewer whose module graph fails to load — a dead page, not a
/// visible error. All four must be present for the viewer to run.
pub(super) const WEB_RUNTIME_ARTIFACTS: [&str; 4] = [
    "clients/web/instrument-runtime.js",
    "clients/web/instrument-runtime_bg.wasm",
    "clients/web/control-runtime.js",
    "clients/web/control-runtime_bg.wasm",
];

/// Ensures the viewer's generated wasm runtime exists AND is current:
/// the build is skipped only when every artifact is present and the content
/// stamp (working-tree source hashes + wasm toolchain versions) matches the
/// one recorded at the last successful build — an edited runtime source or
/// an upgraded wasm-bindgen rebuilds instead of serving a stale viewer.
/// Unlike the camera sidecar this is REQUIRED — the viewer is dead without
/// it — so a build failure aborts the session with the script's actionable
/// toolchain hint (wasm-bindgen / wasm32 target).
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when the build script cannot spawn or fails.
pub(super) fn prepare_web_assets(repo_root: &std::path::Path) -> Result<(), XtaskError> {
    let artifacts_exist = WEB_RUNTIME_ARTIFACTS
        .iter()
        .all(|rel| repo_root.join(rel).is_file());
    let current = stamp::source_stamp(
        repo_root,
        &WEB_RUNTIME_SOURCES,
        &[&["rustc", "--version"], &["wasm-bindgen", "--version"]],
    );
    let stamp_path = repo_root.join(WEB_RUNTIME_STAMP);
    let stored = stamp::read_stamp(&stamp_path);
    if stamp::artifact_is_fresh(artifacts_exist, stored.as_deref(), current.as_deref()) {
        return Ok(());
    }
    print_line(if artifacts_exist {
        "viewer wasm runtime is stale (source or toolchain changed); rebuilding..."
    } else {
        "building the viewer wasm runtime (first run)..."
    });
    let status = Command::new("bash")
        .arg(repo_root.join("scripts/build-web-instruments.sh"))
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "running scripts/build-web-instruments.sh",
            source,
        })?;
    if !status.success() {
        return Err(XtaskError::CommandFailed {
            context: "scripts/build-web-instruments.sh (see its output for the toolchain hint)",
            status: status.to_string(),
        });
    }
    if let Some(current) = current {
        stamp::write_stamp(&stamp_path, &current);
    }
    Ok(())
}
