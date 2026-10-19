//! A flight-controller parameter or configuration write persists on the
//! vehicle, and on a real aircraft some are dangerous (PX4's
//! `COM_RC_IN_MODE = 1` disables the RC transmitter). Each adapter
//! therefore writes from exactly one module, which sends a write only
//! with a proof value its simulation profile alone constructs, and only
//! reads under a physical profile.
//!
//! This guard keeps that one module the only one: it scans the adapter
//! and host sources for the write encoders and request kinds, and names
//! every use outside the allow-listed modules. Tests are exempt; they
//! decode writes to prove which profile sends them.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// Source trees scanned recursively, relative to the repository root.
pub(crate) const TREES: &[&str] = &["adapters", "hosts"];

/// The one module per adapter that may write, relative to the root.
pub(crate) const WRITERS: &[&str] = &[
    "adapters/px4/src/pinning.rs",
    "adapters/ardupilot/src/adapter/params.rs",
    "adapters/betaflight/src/link/setup.rs",
];

/// What a write is spelled with: the MAVLink PARAM_SET encoder, message
/// id, and outbound request kind, and the MSP configuration writes.
pub(crate) const WRITE_TOKENS: &[&str] = &[
    "encode_param_set",
    "PARAM_SET_ID",
    "ParamRequest::Set",
    "encode_set_mode_range",
    "MSP_SET_MODE_RANGE",
    "MSP_EEPROM_WRITE",
];

/// One line that spells a vehicle write outside the allow-listed modules.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// Whether one source line spells a vehicle write.
pub(crate) fn spells_a_write(line: &str) -> bool {
    WRITE_TOKENS.iter().any(|token| line.contains(token))
}

/// Every finding under the scanned trees of `repo_root`.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when a tree cannot be read. A missing tree
/// is an error, not a clean result: the guard then looks at the wrong
/// place.
pub(crate) fn scan(repo_root: &Path) -> Result<Vec<Finding>, XtaskError> {
    let writers: Vec<PathBuf> = WRITERS.iter().map(|path| repo_root.join(path)).collect();
    let mut findings = Vec::new();
    for tree in TREES {
        scan_dir(repo_root, &repo_root.join(tree), &writers, &mut findings)?;
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok(findings)
}

/// Test sources: a `tests.rs`, anything under a `tests` directory of the
/// repository.
fn is_test_source(repo_root: &Path, path: &Path) -> bool {
    let inside = path.strip_prefix(repo_root).unwrap_or(path);
    inside.file_name().is_some_and(|name| name == "tests.rs")
        || inside.components().any(|part| part.as_os_str() == "tests")
}

fn scan_dir(
    repo_root: &Path,
    dir: &Path,
    writers: &[PathBuf],
    findings: &mut Vec<Finding>,
) -> Result<(), XtaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| XtaskError::Io {
        context: "reading a source tree for the vehicle-writes guard",
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| XtaskError::Io {
                context: "reading a source tree entry",
                source,
            })?
            .path();
        if path.is_dir() {
            // Build output is not source.
            if path
                .file_name()
                .is_some_and(|name| name != "target" && name != "build")
            {
                scan_dir(repo_root, &path, writers, findings)?;
            }
            continue;
        }
        let rust = path.extension().is_some_and(|ext| ext == "rs");
        if !rust || is_test_source(repo_root, &path) || writers.contains(&path) {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|source| XtaskError::Io {
            context: "reading a source file for the vehicle-writes guard",
            source,
        })?;
        for (index, line) in text.lines().enumerate() {
            if spells_a_write(line) {
                findings.push(Finding {
                    path: path.clone(),
                    line: index.wrapping_add(1),
                    text: line.trim().to_owned(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
