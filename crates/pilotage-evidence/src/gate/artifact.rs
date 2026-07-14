//! Filesystem resolution of result execution-output artifacts.
//!
//! A verification result names its captured run output with an `artifact` (a
//! repo-relative path) and pins that output with an `output-digest`. This
//! resolver reads the committed artifact, hashes it, and requires the hash to
//! equal the declared digest — so a digest can never be a detached string: it
//! must bind to committed, content-addressed run output. A missing artifact, or
//! one whose content does not hash to the declared digest, is a fail-closed
//! finding. Presence of the fields is checked separately, so a result missing
//! them is skipped here to avoid double-reporting.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::node::NodeKind;
use crate::policy::Policy;

/// Resolves every result's execution-output artifact against `repo_root`.
pub(super) fn resolve(
    graph: &Graph,
    policy: &Policy,
    repo_root: &Path,
    findings: &mut Vec<Finding>,
) {
    for id in graph.ids_of_kind(NodeKind::VerificationResult) {
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let (path, expected) = match (
            node.attr(&policy.result_artifact_attr),
            node.attr(&policy.result_output_attr),
        ) {
            (Some(path), Some(expected)) => (path, expected.trim()),
            _ => continue,
        };
        let full = repo_root.join(path);
        match fs::read(&full) {
            Err(_) => findings.push(Finding::new(
                FindingCode::PlaceholderResult,
                Some(id.clone()),
                format!("result {id}: execution-output artifact {path} not found under repo root"),
            )),
            Ok(bytes) => {
                let actual = format!(
                    "sha256:{}",
                    crate::report::hex(&Sha256::digest(&bytes).into())
                );
                if actual != expected {
                    findings.push(Finding::new(
                        FindingCode::PlaceholderResult,
                        Some(id.clone()),
                        format!(
                            "result {id}: artifact {path} hashes to {actual}, not the declared {expected}"
                        ),
                    ));
                }
            }
        }
    }
}
