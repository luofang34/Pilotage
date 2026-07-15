//! Filesystem resolution of result execution-output artifacts.
//!
//! A verification result names its captured run output with an `artifact` (a
//! repo-relative path) and pins that output with an `output-digest`. This
//! resolver first contains the declared path — it must be root-relative, free
//! of parent (`..`) components, and canonicalize (after symlinks) inside the
//! evidence root — so a result can never bind evidence to a file outside the
//! tree. It then reads the committed artifact, hashes it, and requires the
//! hash to equal the declared digest — so a digest can never be a detached
//! string: it must bind to committed, content-addressed run output. It then
//! parses the artifact as a structured run record and requires its command,
//! configuration digest, tool version, run identity, and outcome to MATCH the
//! graph result node: hash equality alone is not evidence the run happened
//! with the declared command/config/tool as the declared run and produced the
//! declared outcome, and a source file (with no run-record fields) can never
//! satisfy it. An escaping or missing artifact, a content/digest mismatch, an
//! unstructured file, or any disagreeing field is a fail-closed finding.
//! Presence of the fields is checked separately, so a result missing them is
//! skipped here to avoid double-reporting.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::Node;
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
        let full = match crate::gate::contained::resolve_contained(repo_root, path) {
            Ok(full) => full,
            Err(escape) => {
                push(
                    findings,
                    id,
                    format!("execution-output artifact {}", escape.detail(path)),
                );
                continue;
            }
        };
        let bytes = match fs::read(&full) {
            Ok(bytes) => bytes,
            Err(_) => {
                push(
                    findings,
                    id,
                    format!("execution-output artifact {path} not found under repo root"),
                );
                continue;
            }
        };
        let actual = format!(
            "sha256:{}",
            crate::report::hex(&Sha256::digest(&bytes).into())
        );
        if actual != expected {
            push(
                findings,
                id,
                format!("artifact {path} hashes to {actual}, not the declared {expected}"),
            );
        }
        let text = String::from_utf8_lossy(&bytes);
        for problem in field_problems(&text, node, policy, path) {
            push(findings, id, problem);
        }
    }
}

/// Records a placeholder-result finding against `id`.
fn push(findings: &mut Vec<Finding>, id: &NodeId, detail: String) {
    findings.push(Finding::new(
        FindingCode::PlaceholderResult,
        Some(id.clone()),
        format!("result {id}: {detail}"),
    ));
}

/// The ways the artifact's parsed run-record fields disagree with the node, or
/// the single way it is not a structured run record at all.
fn field_problems(text: &str, node: &Node, policy: &Policy, path: &str) -> Vec<String> {
    policy
        .result_artifact_fields
        .iter()
        .filter_map(|field| match (node.attr(field), field_value(text, field)) {
            (Some(declared), Some(found)) if declared == found => None,
            (Some(declared), Some(found)) => Some(format!(
                "artifact {path} {field} {found:?} does not match the result's {declared:?}"
            )),
            (_, None) => Some(format!(
                "artifact {path} is not a structured run record: no {field} field"
            )),
            (None, _) => None,
        })
        .collect()
}

/// The value of a `<field>: <value>` line in a structured run record.
fn field_value(text: &str, field: &str) -> Option<String> {
    let key = format!("{field}:");
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&key)
            .map(|value| value.trim().to_string())
    })
}
