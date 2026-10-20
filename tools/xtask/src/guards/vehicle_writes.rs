//! A flight-controller parameter or configuration write persists on the
//! vehicle, and on a real aircraft some are dangerous (PX4's
//! `COM_RC_IN_MODE = 1` disables the RC transmitter). Each adapter
//! therefore writes from exactly one module, which sends a write only
//! with a proof value its simulation profile alone constructs, and only
//! reads under a physical profile.
//!
//! The simulation backends also set parameters, at start-up of the SITL
//! binary they spawn (PX4's `PX4_PARAM_` environment, ArduPilot's
//! `--defaults` file). Those reach only that SITL process, so their gate
//! is the backend module that spawns it: one per flight-controller stack.
//!
//! This guard keeps those modules the only ones: per [`Rule`], it scans
//! the source trees for the write spellings and names every use outside
//! the allow-listed modules. Tests and comments are exempt; tests decode
//! writes to prove which profile sends them, and a comment writes
//! nothing.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// Where one kind of write may be spelled, and how it is spelled.
#[derive(Debug)]
pub(crate) struct Rule {
    /// Source trees scanned recursively, relative to the repository root.
    pub(crate) trees: &'static [&'static str],
    /// The modules that may write, relative to the root.
    pub(crate) writers: &'static [&'static str],
    /// What a write is spelled with.
    pub(crate) tokens: &'static [&'static str],
}

/// Writes over a vehicle link: the MAVLink PARAM_SET encoder, message
/// id, and outbound request kind, and the MSP configuration writes.
pub(crate) const LINK: Rule = Rule {
    trees: &["adapters", "hosts"],
    writers: &[
        "adapters/px4/src/pinning.rs",
        "adapters/ardupilot/src/adapter/params.rs",
        "adapters/betaflight/src/link/setup.rs",
    ],
    tokens: &[
        "encode_param_set",
        "PARAM_SET_ID",
        "ParamRequest::Set",
        "encode_set_mode_range",
        "MSP_SET_MODE_RANGE",
        "MSP_EEPROM_WRITE",
    ],
};

/// Parameters a simulation backend hands its SITL binary at start-up.
pub(crate) const SITL_LAUNCH: Rule = Rule {
    trees: &["tools", "sim"],
    writers: &[
        "tools/xtask/src/backend/px4_jsbsim.rs",
        "tools/xtask/src/backend/ardupilot_sitl.rs",
    ],
    tokens: &["PX4_PARAM_", "\"--defaults\""],
};

/// Every rule the guard enforces.
pub(crate) const RULES: &[&Rule] = &[&LINK, &SITL_LAUNCH];

/// This guard's own sources spell every token as data.
const SELF: &str = "tools/xtask/src/guards";

/// One line that spells a vehicle write outside the allow-listed modules.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) text: String,
}

impl Rule {
    /// Whether one source line spells a write. A comment line does not.
    pub(crate) fn spells_a_write(&self, line: &str) -> bool {
        !line.trim_start().starts_with("//") && self.tokens.iter().any(|token| line.contains(token))
    }
}

/// Every finding of every rule under `repo_root`.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when a tree cannot be read. A missing tree
/// is an error, not a clean result: the guard then looks at the wrong
/// place.
pub(crate) fn scan(repo_root: &Path) -> Result<Vec<Finding>, XtaskError> {
    let mut findings = Vec::new();
    for rule in RULES {
        findings.extend(scan_rule(repo_root, rule)?);
    }
    Ok(findings)
}

/// Every finding of `rule` under `repo_root`.
///
/// # Errors
///
/// As [`scan`].
pub(crate) fn scan_rule(repo_root: &Path, rule: &Rule) -> Result<Vec<Finding>, XtaskError> {
    let scan = Scan {
        repo_root,
        rule,
        writers: rule
            .writers
            .iter()
            .map(|path| repo_root.join(path))
            .collect(),
        exempt: repo_root.join(SELF),
    };
    let mut findings = Vec::new();
    for tree in rule.trees {
        scan.dir(&repo_root.join(tree), &mut findings)?;
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

struct Scan<'a> {
    repo_root: &'a Path,
    rule: &'a Rule,
    writers: Vec<PathBuf>,
    exempt: PathBuf,
}

impl Scan<'_> {
    fn dir(&self, dir: &Path, findings: &mut Vec<Finding>) -> Result<(), XtaskError> {
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
                if path != self.exempt
                    && path
                        .file_name()
                        .is_some_and(|name| name != "target" && name != "build")
                {
                    self.dir(&path, findings)?;
                }
                continue;
            }
            let rust = path.extension().is_some_and(|ext| ext == "rs");
            if !rust || is_test_source(self.repo_root, &path) || self.writers.contains(&path) {
                continue;
            }
            self.file(path, findings)?;
        }
        Ok(())
    }

    fn file(&self, path: PathBuf, findings: &mut Vec<Finding>) -> Result<(), XtaskError> {
        let text = std::fs::read_to_string(&path).map_err(|source| XtaskError::Io {
            context: "reading a source file for the vehicle-writes guard",
            source,
        })?;
        for (index, line) in text.lines().enumerate() {
            if self.rule.spells_a_write(line) {
                findings.push(Finding {
                    path: path.clone(),
                    line: index.wrapping_add(1),
                    text: line.trim().to_owned(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
