//! Filesystem resolution of the specific review-record entry a review names.
//!
//! A review that *claims* completion names its record with a `locator` of the
//! form `path#anchor`. This resolver locates that one entry — `<a id="anchor">`
//! within the file — and verifies THAT entry's fields: a real reviewer and date
//! (not the project's `PENDING` marker), a closed disposition, and coverage of
//! every node the review reviews. It never accepts a file-wide match: a review
//! whose named entry is missing or incomplete fails even if the file holds other
//! complete entries. Non-completed reviews are flagged structurally elsewhere
//! and skipped here.

use std::fs;
use std::path::Path;

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::NodeKind;
use crate::policy::Policy;
use crate::relation::RelationKind;

/// The project's marker for an unfinished record field (review-record.md).
const INCOMPLETE_MARKER: &str = "pending";

/// Resolves every completed review's named record entry against `repo_root`.
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
        let locator = match graph.node(id).and_then(|n| n.locator.as_deref()) {
            Some(locator) if !locator.is_empty() => locator,
            _ => continue,
        };
        for problem in entry_problems(graph, policy, id, locator, repo_root) {
            findings.push(Finding::new(
                FindingCode::ReviewIncomplete,
                Some(id.clone()),
                format!("review {id} {problem}"),
            ));
        }
    }
}

/// Resolves the named entry and returns every reason it is not a complete,
/// covering record — or a single reason it could not be resolved at all.
fn entry_problems(
    graph: &Graph,
    policy: &Policy,
    id: &NodeId,
    locator: &str,
    repo_root: &Path,
) -> Vec<String> {
    let Some((path, anchor)) = locator.split_once('#') else {
        return vec![format!(
            "is marked complete but its locator names no specific record entry (expected path#anchor)"
        )];
    };
    let full = match crate::gate::contained::resolve_contained(repo_root, path) {
        Ok(full) => full,
        Err(escape) => return vec![format!("record file {}", escape.detail(path))],
    };
    let text = match fs::read_to_string(&full) {
        Ok(text) => text,
        Err(_) => return vec![format!("record file {path} not found under repo root")],
    };
    let Some(entry) = entry_block(&text, anchor) else {
        return vec![format!("record entry #{anchor} not found in {path}")];
    };
    field_problems(&entry, policy, &reviewed_ids(graph, id))
        .into_iter()
        .map(|problem| format!("record entry #{anchor}: {problem}"))
        .collect()
}

/// The text of the entry introduced by `<a id="anchor">`, up to the next anchor.
fn entry_block(text: &str, anchor: &str) -> Option<String> {
    let needle = format!("id=\"{anchor}\"");
    let start = text.find(&needle)?;
    let rest = &text[start + needle.len()..];
    let end = rest.find("<a id=").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// The problems with a named entry: missing/pending reviewer or date, a
/// non-closed disposition, or a reviewed node the entry does not cover.
fn field_problems(entry: &str, policy: &Policy, reviewed: &[NodeId]) -> Vec<String> {
    let mut problems = Vec::new();
    for field in ["reviewer", "date"] {
        match field_value(entry, field) {
            Some(value) if !value.eq_ignore_ascii_case(INCOMPLETE_MARKER) => {}
            _ => problems.push(format!("{field} is missing or pending")),
        }
    }
    match field_value(entry, "disposition") {
        Some(value) if is_closed(&value, policy) => {}
        Some(value) => problems.push(format!("disposition '{value}' is not a closed outcome")),
        None => problems.push("disposition is missing".to_string()),
    }
    let covers = field_value(entry, "covers").unwrap_or_default();
    for node in reviewed {
        if !covers_node(&covers, node.as_str()) {
            problems.push(format!("does not cover reviewed node {node}"));
        }
    }
    problems
}

/// Whether `covers` names `id` as a whole identifier token. Tolerates markdown
/// link syntax (`[`ID`](path)`) by splitting on any non-identifier character, so
/// the record can link requirement ids for the human-facing guards.
fn covers_node(covers: &str, id: &str) -> bool {
    covers
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-')))
        .any(|token| token == id)
}

/// Whether `disposition` is one of the policy's closed outcomes.
fn is_closed(disposition: &str, policy: &Policy) -> bool {
    let normalized = disposition.trim().to_ascii_lowercase();
    policy.review_closed_dispositions.contains(&normalized)
}

/// The value of a `- <field>: <value>` (or `<field>: <value>`) line in `entry`.
fn field_value(entry: &str, field: &str) -> Option<String> {
    let key = format!("{field}:");
    entry.lines().find_map(|line| {
        let line = line.trim().trim_start_matches('-').trim();
        line.strip_prefix(&key)
            .map(|value| value.trim().to_string())
    })
}

/// The ids of nodes this review reviews.
fn reviewed_ids(graph: &Graph, id: &NodeId) -> Vec<NodeId> {
    graph
        .edges()
        .filter(|e| e.from == *id && e.relation == RelationKind::Reviews)
        .map(|e| e.to.clone())
        .collect()
}

/// Whether the review reviews something and claims a completed status.
fn claims_completion(graph: &Graph, policy: &Policy, id: &NodeId) -> bool {
    let reviews = graph
        .edges()
        .any(|e| e.from == *id && e.relation == RelationKind::Reviews);
    let status = graph
        .node(id)
        .and_then(|n| n.attr("status"))
        .unwrap_or_default();
    reviews && policy.review_complete_statuses.iter().any(|s| s == status)
}
