//! Fail-closed resolution of declared root-relative paths.
//!
//! Every path a graph declares (execution-output artifact, review record,
//! verification-case locator) resolves through here: the declared path must be
//! relative, contain no parent (`..`) component, resolve to a real file, and
//! canonicalize — after following symlinks — to a location still inside the
//! canonical root. A declared path can therefore never read a file outside the
//! evidence root, whether by absolute path, traversal, or symlink escape.

use std::path::{Component, Path, PathBuf};

/// Why a declared path was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathEscape {
    /// The declared path is absolute.
    Absolute,
    /// The declared path contains a parent (`..`) component.
    ParentTraversal,
    /// The path does not resolve to a readable file under the root.
    Unresolvable,
    /// The canonical target (after symlinks) lies outside the canonical root.
    OutsideRoot,
}

impl PathEscape {
    /// The finding-detail phrase for this refusal, starting with the declared
    /// path so callers prefix their own subject ("artifact …", "file …").
    pub(super) fn detail(self, declared: &str) -> String {
        match self {
            Self::Absolute => {
                format!("{declared} is an absolute path; declared paths must be root-relative")
            }
            Self::ParentTraversal => {
                format!("{declared} contains a parent ('..') path component")
            }
            Self::Unresolvable => format!("{declared} not found under repo root"),
            Self::OutsideRoot => {
                format!("{declared} resolves outside the evidence root (symlink escape)")
            }
        }
    }
}

/// Canonicalizes `declared` under `root` and requires the result to stay
/// inside it, failing closed on every escape.
pub(super) fn resolve_contained(root: &Path, declared: &str) -> Result<PathBuf, PathEscape> {
    let path = Path::new(declared);
    if path.is_absolute() {
        return Err(PathEscape::Absolute);
    }
    if path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(PathEscape::ParentTraversal);
    }
    let root = root.canonicalize().map_err(|_| PathEscape::Unresolvable)?;
    let full = root
        .join(path)
        .canonicalize()
        .map_err(|_| PathEscape::Unresolvable)?;
    if full.starts_with(&root) {
        Ok(full)
    } else {
        Err(PathEscape::OutsideRoot)
    }
}
