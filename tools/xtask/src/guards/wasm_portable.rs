//! The crates a browser or simulator host links into its own build stay
//! portable: each one, without its default features, compiles for
//! `wasm32-unknown-unknown`, and its normal dependency tree for that
//! target holds no socket runtime (tokio, mio, socket2).

use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// The guard's name in the guard report.
pub(crate) const NAME: &str = "wasm-portable-crates";

/// The target every portable crate builds for.
const TARGET: &str = "wasm32-unknown-unknown";

/// The packages that must stay portable without their default features.
pub(crate) const PORTABLE_CRATES: &[&str] = &["pilotage-mavlink"];

/// The crates a portable dependency tree must not hold.
pub(crate) const FORBIDDEN_CRATES: &[&str] = &["tokio", "mio", "socket2"];

/// Checks every portable crate and returns one line for each refusal.
///
/// # Errors
///
/// [`XtaskError::Spawn`] when cargo does not start.
pub(crate) fn check(repo_root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut refusals = Vec::new();
    for package in PORTABLE_CRATES {
        let build = cargo(
            repo_root,
            &["check", "-p", package, "--no-default-features"],
        )?;
        if !build.status.success() {
            refusals.push(format!(
                "FORBIDDEN: {package} without default features does not build for {TARGET}:\n{}",
                String::from_utf8_lossy(&build.stderr)
            ));
        }
        let tree = cargo(
            repo_root,
            &[
                "tree",
                "-p",
                package,
                "--no-default-features",
                "-e",
                "normal",
                "--prefix",
                "none",
            ],
        )?;
        if !tree.status.success() {
            refusals.push(format!(
                "FORBIDDEN: cargo tree for {package} failed:\n{}",
                String::from_utf8_lossy(&tree.stderr)
            ));
            continue;
        }
        for name in forbidden_in_tree(&String::from_utf8_lossy(&tree.stdout)) {
            refusals.push(format!(
                "FORBIDDEN: {package} without default features depends on {name} for {TARGET}"
            ));
        }
    }
    Ok(refusals)
}

/// The forbidden crates that a `cargo tree --prefix none` listing names,
/// each once, in [`FORBIDDEN_CRATES`] order.
pub(crate) fn forbidden_in_tree(listing: &str) -> Vec<&'static str> {
    FORBIDDEN_CRATES
        .iter()
        .copied()
        .filter(|forbidden| {
            listing
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .any(|name| name == *forbidden)
        })
        .collect()
}

fn cargo(repo_root: &Path, args: &[&str]) -> Result<std::process::Output, XtaskError> {
    Command::new("cargo")
        .args(args)
        .args(["--target", TARGET])
        .current_dir(repo_root)
        .output()
        .map_err(|source| XtaskError::Spawn {
            name: "cargo for the wasm-portable guard",
            source,
        })
}
