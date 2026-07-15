//! Filesystem resolution of the test-source binding a recorded result claims.
//!
//! A verification result pins the exact test source its recorded run executed
//! with a `source-digest` (`git-blob:<sha1>`, git's object id for the file).
//! This resolver walks the result's `result-of` cases, hashes each distinct
//! case locator file under the contained root, and requires every hash to
//! equal the declared digest. The selector check binds the test *name* to the
//! current tree; this binds the recorded result to the test *content* — so a
//! result recorded against an older implementation of the same-named test
//! fails closed instead of silently passing. A non-`git-blob` scheme, an
//! escaping or unreadable locator, or any disagreeing hash is a fail-closed
//! finding. Presence of the attribute is checked structurally, so a result
//! missing it is skipped here to avoid double-reporting.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha1::{Digest, Sha1};

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::NodeKind;
use crate::policy::Policy;
use crate::relation::RelationKind;

/// The digest scheme naming a git blob object id.
const GIT_BLOB_SCHEME: &str = "git-blob:";

/// Resolves every result's declared test-source digest against `repo_root`.
pub(super) fn resolve(
    graph: &Graph,
    policy: &Policy,
    repo_root: &Path,
    findings: &mut Vec<Finding>,
) {
    for id in graph.ids_of_kind(NodeKind::VerificationResult) {
        let Some(declared) = graph
            .node(id)
            .and_then(|n| n.attr(&policy.result_source_attr))
        else {
            continue;
        };
        let Some(expected) = declared.trim().strip_prefix(GIT_BLOB_SCHEME) else {
            push(
                findings,
                id,
                format!("source-digest {declared} does not use the {GIT_BLOB_SCHEME}<sha1> scheme"),
            );
            continue;
        };
        for locator in case_locators(graph, id) {
            let path = locator.split('#').next().unwrap_or(locator);
            check_locator(repo_root, path, expected, findings, id);
        }
    }
}

/// Hashes one case locator file and reports any escape, read failure, or
/// digest disagreement.
fn check_locator(
    repo_root: &Path,
    path: &str,
    expected: &str,
    findings: &mut Vec<Finding>,
    id: &NodeId,
) {
    let full = match crate::gate::contained::resolve_contained(repo_root, path) {
        Ok(full) => full,
        Err(escape) => {
            push(findings, id, format!("test source {}", escape.detail(path)));
            return;
        }
    };
    let bytes = match fs::read(&full) {
        Ok(bytes) => bytes,
        Err(_) => {
            push(
                findings,
                id,
                format!("test source {path} not found under repo root"),
            );
            return;
        }
    };
    let actual = git_blob_sha1(&bytes);
    if actual != expected {
        push(
            findings,
            id,
            format!(
                "source-digest {GIT_BLOB_SCHEME}{expected} does not match the current test \
                 source {path} ({GIT_BLOB_SCHEME}{actual}); the recorded result predates the \
                 test source"
            ),
        );
    }
}

/// The distinct locators of the cases this result records (`result-of`).
fn case_locators<'a>(graph: &'a Graph, id: &NodeId) -> BTreeSet<&'a str> {
    graph
        .edges()
        .filter(|e| e.from == *id && e.relation == RelationKind::ResultOf)
        .filter_map(|e| graph.node(&e.to))
        .filter(|n| n.kind == NodeKind::VerificationCase)
        .filter_map(|n| n.locator.as_deref())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Git's object id for a blob: SHA-1 over `blob <len>\0<bytes>`.
fn git_blob_sha1(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(b"blob ");
    hasher.update(bytes.len().to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Records a placeholder-result finding against `id`.
fn push(findings: &mut Vec<Finding>, id: &NodeId, detail: String) {
    findings.push(Finding::new(
        FindingCode::PlaceholderResult,
        Some(id.clone()),
        format!("result {id}: {detail}"),
    ));
}
