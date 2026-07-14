//! Filesystem resolution of review-record entries.
//!
//! A review node that *claims* a completed status names its substantive record
//! with a `locator` (a repo-relative path, optionally with a `#anchor`). This
//! resolver reads that record and confirms it is not still pending, so a review
//! cannot be marked complete on the node while its underlying record — reviewer,
//! date, disposition, closure — is absent or unfinished. A missing record or a
//! record still carrying the project's `PENDING` incomplete marker is a
//! fail-closed finding, never a silent pass.

use std::fs;
use std::path::Path;

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::node::NodeKind;
use crate::policy::Policy;
use crate::relation::RelationKind;

/// The project's marker for an unfinished record field (see
/// `docs/instruments/review-record.md`): its presence means the record is not
/// complete.
const INCOMPLETE_MARKER: &str = "pending";

/// Resolves every completed review's record against `repo_root`.
pub(super) fn resolve(
    graph: &Graph,
    policy: &Policy,
    repo_root: &Path,
    findings: &mut Vec<Finding>,
) {
    for id in graph.ids_of_kind(NodeKind::Review) {
        if !claims_completion(graph, policy, id) {
            continue;
        }
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let locator = match node.locator.as_deref().filter(|p| !p.is_empty()) {
            Some(locator) => locator,
            None => continue,
        };
        let path = locator.split('#').next().unwrap_or(locator);
        let full = repo_root.join(path);
        match fs::read_to_string(&full) {
            Err(_) => findings.push(Finding::new(
                FindingCode::ReviewIncomplete,
                Some(id.clone()),
                format!("review {id}: record file {path} not found under repo root"),
            )),
            Ok(text) if text.to_ascii_lowercase().contains(INCOMPLETE_MARKER) => {
                findings.push(Finding::new(
                    FindingCode::ReviewIncomplete,
                    Some(id.clone()),
                    format!(
                        "review {id} is marked complete, but its record {path} still has {INCOMPLETE_MARKER} fields"
                    ),
                ));
            }
            Ok(_) => {}
        }
    }
}

/// Whether the review reviews something and claims a completed status.
fn claims_completion(graph: &Graph, policy: &Policy, id: &crate::id::NodeId) -> bool {
    let reviews = graph
        .edges()
        .any(|e| e.from == *id && e.relation == RelationKind::Reviews);
    let status = graph
        .node(id)
        .and_then(|n| n.attr("status"))
        .unwrap_or_default();
    reviews && policy.review_complete_statuses.iter().any(|s| s == status)
}
