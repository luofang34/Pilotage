//! Answers which halves of the CI matrix a diff can reach.
//!
//! The declarations live in Cargo metadata: the workspace manifest names
//! each domain's paths and root packages, and the build graph supplies
//! the reach of every Rust change. The classifier fails OPEN — a file it
//! cannot place, a graph it cannot read, or a change to the classifier's
//! own inputs answers "run everything" — so an uncertain selector can
//! only ever cost time, never coverage.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

mod model;
#[cfg(test)]
mod tests;

use model::{ClassifierModel, Domain};

/// Runs the classification against `base...HEAD` and prints one
/// `key=value` line per answer, with `# `-prefixed reason lines above
/// them so a CI log shows WHY each answer holds.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when git or cargo cannot answer; the
/// CI caller treats any failure as "run everything".
pub fn run_affected(base: &str) -> Result<(), XtaskError> {
    let files = changed_files(base)?;
    let metadata = load_metadata()?;
    let model = ClassifierModel::from_workspace(&metadata)?;
    let outcome = classify(&files, &model);
    for reason in &outcome.reasons {
        print_line(&format!("# {reason}"));
    }
    print_line(&format!("everything={}", outcome.everything));
    for (domain, affected) in &outcome.domains {
        print_line(&format!("{domain}={}", outcome.everything || *affected));
    }
    Ok(())
}

/// One classification result: the global answer, each domain's answer,
/// and the chain of reasons behind them.
pub(crate) struct Outcome {
    pub(crate) everything: bool,
    pub(crate) domains: BTreeMap<String, bool>,
    pub(crate) reasons: Vec<String>,
}

/// Paths whose change invalidates the classifier's own judgment: its
/// sources, the workflows that consume it, and the manifests and lock
/// that define the graph it reads.
fn distrusts_itself(file: &str) -> bool {
    file.starts_with("tools/xtask/")
        || file.starts_with(".github/")
        || file == "Cargo.lock"
        || file == "Cargo.toml"
        || file.ends_with("/Cargo.toml")
}

pub(crate) fn classify(files: &[String], model: &ClassifierModel) -> Outcome {
    let mut outcome = Outcome {
        everything: false,
        domains: model
            .domains
            .keys()
            .map(|name| (name.clone(), false))
            .collect(),
        reasons: Vec::new(),
    };
    for file in files {
        classify_one(file, model, &mut outcome);
    }
    outcome
}

fn classify_one(file: &str, model: &ClassifierModel, outcome: &mut Outcome) {
    if distrusts_itself(file) {
        outcome.everything = true;
        outcome
            .reasons
            .push(format!("{file}: classifier input changed, everything runs"));
        return;
    }
    let mut placed = false;
    for (name, domain) in &model.domains {
        if domain_claims_path(domain, file) {
            outcome.domains.insert(name.clone(), true);
            outcome.reasons.push(format!("{file}: {name} path"));
            placed = true;
        }
    }
    if let Some(package) = model.owning_package(file) {
        placed = true;
        for (name, domain) in &model.domains {
            if domain.package_closure.contains(package) {
                outcome.domains.insert(name.clone(), true);
                outcome
                    .reasons
                    .push(format!("{file}: package {package} is in {name}'s closure"));
            }
        }
    }
    if model.is_inert(file) {
        placed = true;
    }
    if !placed {
        outcome.everything = true;
        outcome
            .reasons
            .push(format!("{file}: no declaration places it, everything runs"));
    }
}

fn domain_claims_path(domain: &Domain, file: &str) -> bool {
    domain
        .paths
        .iter()
        .chain(domain.extra_paths.iter())
        .any(|prefix| file.starts_with(prefix.as_str()) || file == prefix.trim_end_matches('/'))
}

fn changed_files(base: &str) -> Result<Vec<String>, XtaskError> {
    let output = Command::new("git")
        .args(["diff", "--name-only", &format!("{base}...HEAD")])
        .output()
        .map_err(|source| XtaskError::Io {
            context: "running git diff for the change classification",
            source,
        })?;
    if !output.status.success() {
        return Err(XtaskError::Usage {
            message: format!(
                "git diff {base}...HEAD failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .filter(|line| !line.is_empty())
        .collect())
}

fn load_metadata() -> Result<serde_json::Value, XtaskError> {
    // `--no-deps` still carries every package's DECLARED dependency
    // list, which is all the closure needs — and it resolves nothing,
    // so the classifier runs without the credentials the workspace's
    // private git dependencies would demand.
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|source| XtaskError::Io {
            context: "running cargo metadata for the change classification",
            source,
        })?;
    if !output.status.success() {
        return Err(XtaskError::Usage {
            message: format!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|source| XtaskError::Usage {
        message: format!("cargo metadata produced unreadable JSON: {source}"),
    })
}

/// The longest-prefix owner test used by the model: `dir` owns `file`
/// when the file sits inside it.
pub(crate) fn dir_owns(dir: &Path, file: &str) -> bool {
    Path::new(file).starts_with(dir)
}
