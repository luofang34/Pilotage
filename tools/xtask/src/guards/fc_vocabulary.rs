//! Clients speak the protocol's typed vocabulary and never a flight
//! controller's own codes (ADR-0008): the adapter translates, so a new
//! flight-controller family needs no client change.
//!
//! This guard scans the client-side sources for flight-controller
//! vocabulary — MAVLink names and constants — and names every hit.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// Client-side source trees, relative to the repository root. Each is
/// scanned recursively for `.rs`, `.js`, and `.mjs` files.
pub(crate) const CLIENT_TREES: &[&str] = &[
    "crates/pilotage-control/src",
    "crates/pilotage-client-session/src",
    "crates/pilotage-input/src",
    "crates/pilotage-protocol/src",
    "crates/pilotage-instrument-feed/src",
    "clients/web-instrument-shell/src",
    "clients/apple/rust/pilotage-situation-ffi/src",
];

/// Client directories scanned without recursion: their subdirectories
/// hold vendored or generated code.
pub(crate) const CLIENT_FLAT_DIRS: &[&str] = &["clients/web"];

/// One line that carries flight-controller vocabulary.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// Whether one source line names flight-controller vocabulary.
pub(crate) fn names_fc_vocabulary(line: &str) -> bool {
    line.contains("MAV_")
        || line.contains("GIMBAL_DEVICE_FLAGS")
        || line.to_ascii_lowercase().contains("mavlink")
}

/// Every finding under the client trees of `repo_root`.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when a client tree cannot be read. A
/// missing tree is an error, not a clean result: the guard then looks at
/// the wrong place.
pub(crate) fn scan(repo_root: &Path) -> Result<Vec<Finding>, XtaskError> {
    let mut findings = Vec::new();
    for tree in CLIENT_TREES {
        scan_dir(&repo_root.join(tree), true, &mut findings)?;
    }
    for dir in CLIENT_FLAT_DIRS {
        scan_dir(&repo_root.join(dir), false, &mut findings)?;
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok(findings)
}

fn scan_dir(dir: &Path, recurse: bool, findings: &mut Vec<Finding>) -> Result<(), XtaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| XtaskError::Io {
        context: "reading a client source tree for the FC-vocabulary guard",
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| XtaskError::Io {
                context: "reading a client source tree entry",
                source,
            })?
            .path();
        if path.is_dir() {
            if recurse {
                scan_dir(&path, true, findings)?;
            }
            continue;
        }
        let scanned = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "rs" | "js" | "mjs"));
        if !scanned {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|source| XtaskError::Io {
            context: "reading a client source file",
            source,
        })?;
        for (index, line) in text.lines().enumerate() {
            if names_fc_vocabulary(line) {
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
