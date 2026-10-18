//! Runs every guard the repository declares, each proven by its own
//! self-test first.
//!
//! A guard is a pair by convention: `scripts/check-<name>.sh` and its
//! sibling `scripts/test-check-<name>.sh`. The runner discovers the
//! pairs instead of reading a list, so adding a guard never edits CI —
//! and a guard whose self-test is missing stays a manual step until it
//! earns the pairing. Every pair runs even after one fails, so a red
//! run names every failing guard at once.
//!
//! A guard written in Rust is a function in a submodule here, proven
//! by its unit tests. Each runs after the pairs.

mod aviate_control_feel;
mod pipefail_pipelines;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

mod fc_vocabulary;

#[cfg(test)]
mod tests;

/// Discovers and runs every guard pair under `scripts/`, then every
/// Rust guard.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] naming every failing guard, or the
/// discovery failure when the scripts directory cannot be read.
pub fn run_guards(repo_root: &Path) -> Result<(), XtaskError> {
    let scripts = repo_root.join("scripts");
    let pairs = discover_pairs(&scripts)?;
    if pairs.is_empty() {
        return Err(XtaskError::NoGuardPairs { scripts });
    }
    let mut failed = Vec::new();
    for pair in &pairs {
        let outcome = run_pair(repo_root, pair);
        match outcome {
            Ok(()) => print_line(&format!("guard {}: ok", pair.name)),
            Err(stage) => {
                print_line(&format!("guard {}: FAILED at {stage}", pair.name));
                failed.push(pair.name.clone());
            }
        }
    }
    run_rust_guards(repo_root, &mut failed);
    if failed.is_empty() {
        print_line(&format!(
            "guards: {} pairs and the Rust guards ok",
            pairs.len()
        ));
        Ok(())
    } else {
        Err(XtaskError::GuardsFailed { names: failed })
    }
}

fn run_rust_guards(repo_root: &Path, failed: &mut Vec<String>) {
    run_aviate_control_feel(repo_root, failed);
    run_fc_vocabulary(repo_root, failed);
    run_pipefail_pipelines(repo_root, failed);
}

fn run_aviate_control_feel(repo_root: &Path, failed: &mut Vec<String>) {
    let name = "aviate-control-feel-boundary";
    match aviate_control_feel::check(repo_root) {
        Ok(findings) if findings.is_empty() => print_line(&format!("guard {name}: ok")),
        Ok(findings) => {
            for finding in &findings {
                print_line(&format!(
                    "FORBIDDEN: Aviate response constant bypasses the control-feel artifact: {}/{}:{}",
                    aviate_control_feel::AVIATE_SRC,
                    finding.path,
                    finding.name
                ));
            }
            print_line(&format!("guard {name}: FAILED at the check"));
            failed.push(name.to_owned());
        }
        Err(error) => {
            print_line(&format!("guard {name}: FAILED: {error}"));
            failed.push(name.to_owned());
        }
    }
}

fn run_fc_vocabulary(repo_root: &Path, failed: &mut Vec<String>) {
    let name = "fc-vocabulary";
    match fc_vocabulary::scan(repo_root) {
        Ok(findings) if findings.is_empty() => print_line(&format!("guard {name}: ok")),
        Ok(findings) => {
            for finding in &findings {
                print_line(&format!(
                    "FORBIDDEN: flight-controller vocabulary in a client: {}:{}: {}",
                    finding.path.display(),
                    finding.line,
                    finding.text
                ));
            }
            print_line(&format!("guard {name}: FAILED at the check"));
            failed.push(name.to_owned());
        }
        Err(error) => {
            print_line(&format!("guard {name}: FAILED: {error}"));
            failed.push(name.to_owned());
        }
    }
}

fn run_pipefail_pipelines(repo_root: &Path, failed: &mut Vec<String>) {
    let name = "pipefail-pipelines";
    match pipefail_pipelines::check(repo_root) {
        Ok(findings) if findings.is_empty() => print_line(&format!("guard {name}: ok")),
        Ok(findings) => {
            for finding in &findings {
                print_line(&finding.to_string());
            }
            print_line(&format!("guard {name}: FAILED"));
            failed.push(name.to_owned());
        }
        Err(error) => {
            print_line(&format!("guard {name}: FAILED: {error}"));
            failed.push(name.to_owned());
        }
    }
}

/// One discovered guard: the check script and the self-test that
/// proves the check can still fail.
pub(crate) struct GuardPair {
    pub(crate) name: String,
    pub(crate) check: PathBuf,
    pub(crate) self_test: PathBuf,
}

pub(crate) fn discover_pairs(scripts: &Path) -> Result<Vec<GuardPair>, XtaskError> {
    let entries = std::fs::read_dir(scripts).map_err(|source| XtaskError::Io {
        context: "reading the scripts directory for guard discovery",
        source,
    })?;
    let mut pairs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| XtaskError::Io {
            context: "reading a scripts directory entry",
            source,
        })?;
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .and_then(|value| value.strip_prefix("check-"))
            .and_then(|value| value.strip_suffix(".sh"))
        else {
            continue;
        };
        let self_test = scripts.join(format!("test-check-{name}.sh"));
        if self_test.is_file() {
            pairs.push(GuardPair {
                name: name.to_owned(),
                check: entry.path(),
                self_test,
            });
        }
    }
    pairs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pairs)
}

/// Runs the self-test, then the check. The self-test goes first
/// because a guard that cannot fail proves nothing — a check script
/// whose refusal path rotted would pass every tree.
fn run_pair(repo_root: &Path, pair: &GuardPair) -> Result<(), &'static str> {
    if !bash_passes(repo_root, &pair.self_test) {
        return Err("its self-test");
    }
    if !bash_passes(repo_root, &pair.check) {
        return Err("the check");
    }
    Ok(())
}

fn bash_passes(repo_root: &Path, script: &Path) -> bool {
    match Command::new("bash")
        .arg(script)
        .current_dir(repo_root)
        .status()
    {
        Ok(status) => status.success(),
        Err(source) => {
            // A script that never started must not read as a script
            // that refused: name the spawn failure before failing.
            print_line(&format!(
                "guard script {} did not start: {source}",
                script.display()
            ));
            false
        }
    }
}
