//! The individual no-orphan checks.
//!
//! Each check appends findings for its own concern; the orchestrator in the
//! parent module short-circuits the empty-graph case before any of these run,
//! so every check here can assume a non-empty graph and scope.

use std::collections::BTreeSet;

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::{Node, NodeKind};
use crate::policy::Policy;
use crate::relation::{Edge, RelationKind};

/// Every scope requirement must exist and be a requirement node.
pub(super) fn scope_roots(graph: &Graph, findings: &mut Vec<Finding>) {
    for id in &graph.scope.requirements {
        match graph.kind_of(id) {
            None => findings.push(Finding::new(
                FindingCode::MissingScopeRequirement,
                Some(id.clone()),
                format!("scope requirement {id} is not defined as a node"),
            )),
            Some(kind) if !kind.is_requirement() => findings.push(Finding::new(
                FindingCode::MissingScopeRequirement,
                Some(id.clone()),
                format!(
                    "scope requirement {id} is a {} node, not a requirement",
                    kind.token()
                ),
            )),
            Some(_) => {}
        }
    }
}

/// Every edge endpoint must resolve to a node.
pub(super) fn dangling_edges(graph: &Graph, findings: &mut Vec<Finding>) {
    for edge in graph.edges() {
        let missing_from = !graph.contains(&edge.from);
        let missing_to = !graph.contains(&edge.to);
        if missing_from || missing_to {
            let which = match (missing_from, missing_to) {
                (true, true) => format!("both {} and {}", edge.from, edge.to),
                (true, false) => edge.from.to_string(),
                (false, true) => edge.to.to_string(),
                (false, false) => unreachable!(),
            };
            findings.push(Finding::new(
                FindingCode::DanglingEdge,
                None,
                format!(
                    "edge {} {} {} references undefined node {which}",
                    edge.from,
                    edge.relation.token(),
                    edge.to
                ),
            ));
        }
    }
}

/// Every in-scope requirement must trace upstream and downstream.
pub(super) fn requirement_traces(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    for id in &graph.scope.requirements {
        if !graph.kind_of(id).is_some_and(NodeKind::is_requirement) {
            continue;
        }
        upstream(graph, policy, id, findings);
        downstream(graph, policy, id, findings);
    }
}

fn upstream(graph: &Graph, policy: &Policy, id: &NodeId, findings: &mut Vec<Finding>) {
    let reached = graph.reachable_via(id, &policy.upstream_forward, &policy.upstream_reverse);
    let kinds = kinds_present(graph, &reached);
    let by_rationale = policy.upstream_rationale.iter().any(|k| kinds.contains(k));
    let missing: Vec<&str> = policy
        .upstream_required
        .iter()
        .filter(|k| !kinds.contains(k))
        .map(|k| k.token())
        .collect();
    if !by_rationale && !missing.is_empty() {
        findings.push(Finding::new(
            FindingCode::MissingUpstream,
            Some(id.clone()),
            format!("{id} reaches no upstream {}", missing.join(", ")),
        ));
    }
}

fn downstream(graph: &Graph, policy: &Policy, id: &NodeId, findings: &mut Vec<Finding>) {
    let reached = graph.reachable_via(id, &policy.downstream_forward, &policy.downstream_reverse);
    let kinds = kinds_present(graph, &reached);
    let missing: Vec<&str> = policy
        .downstream_required
        .iter()
        .filter(|k| !kinds.contains(k))
        .map(|k| k.token())
        .collect();
    if !missing.is_empty() {
        findings.push(Finding::new(
            FindingCode::MissingDownstreamLayer,
            Some(id.clone()),
            format!("{id} reaches no downstream {}", missing.join(", ")),
        ));
    }
}

/// Every recorded result must resolve to case, baseline, tool, and requirement.
pub(super) fn results(graph: &Graph, findings: &mut Vec<Finding>) {
    for id in graph.ids_of_kind(NodeKind::VerificationResult) {
        let case = target_of_kind(
            graph,
            id,
            RelationKind::ResultOf,
            NodeKind::VerificationCase,
        );
        let mut missing = Vec::new();
        if case.is_none() {
            missing.push("executed case (result-of)".to_string());
        }
        if !reaches_kind(graph, id, NodeKind::ConfigurationItem) {
            missing.push("configuration baseline".to_string());
        }
        if !reaches_kind(graph, id, NodeKind::Tool) {
            missing.push("tool identity".to_string());
        }
        if !covers_scope_requirement(graph, id, case.as_ref()) {
            missing.push("covered in-scope requirement".to_string());
        }
        if !missing.is_empty() {
            findings.push(Finding::new(
                FindingCode::ResultUnresolved,
                Some(id.clone()),
                format!("result {id} does not resolve to {}", missing.join(", ")),
            ));
        }
    }
}

/// Every verification case that covers an in-scope requirement must have a
/// recorded result. A case with no incoming `result-of` from a
/// verification-result is a gap: the trace names a check but records no outcome.
pub(super) fn cases_have_results(graph: &Graph, findings: &mut Vec<Finding>) {
    for id in graph.ids_of_kind(NodeKind::VerificationCase) {
        if !verifies_scope_requirement(graph, id) || has_result(graph, id) {
            continue;
        }
        findings.push(Finding::new(
            FindingCode::MissingResult,
            Some(id.clone()),
            format!(
                "verification case {id} covers an in-scope requirement but has no recorded result"
            ),
        ));
    }
}

/// Whether some verification-result is the recorded outcome of running `case`.
fn has_result(graph: &Graph, case: &NodeId) -> bool {
    graph.edges().any(|e| {
        e.to == *case
            && e.relation == RelationKind::ResultOf
            && graph.kind_of(&e.from) == Some(NodeKind::VerificationResult)
    })
}

/// Whether `case` verifies or covers an in-scope requirement.
fn verifies_scope_requirement(graph: &Graph, case: &NodeId) -> bool {
    graph.edges().any(|e| {
        (e.to == *case
            && e.relation == RelationKind::VerifiedBy
            && graph.scope.requirements.contains(&e.from))
            || (e.from == *case
                && e.relation == RelationKind::Covers
                && graph.scope.requirements.contains(&e.to))
    })
}

/// Every recorded result must carry immutable provenance: the executed command,
/// the configuration commit/tree digest, the pinned tool version, an immutable
/// execution-output digest of the captured run, and the run identity. A result
/// missing any of these — or whose execution-output digest merely points at the
/// test source rather than a captured run output — is a placeholder, not
/// evidence, and must fail the gate.
pub(super) fn result_provenance(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    for id in graph.ids_of_kind(NodeKind::VerificationResult) {
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let missing: Vec<&str> = policy
            .result_required_attrs
            .iter()
            .filter(|attr| node.attr(attr).is_none())
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            findings.push(Finding::new(
                FindingCode::PlaceholderResult,
                Some(id.clone()),
                format!(
                    "result {id} is a placeholder: missing {}",
                    missing.join(", ")
                ),
            ));
            continue;
        }
        if let Some(output) = node.attr(&policy.result_output_attr)
            && is_source_blob(output)
        {
            findings.push(Finding::new(
                FindingCode::PlaceholderResult,
                Some(id.clone()),
                format!(
                    "result {id} is a placeholder: {} {output} references the test source, not a captured execution output",
                    policy.result_output_attr
                ),
            ));
        }
    }
}

/// Whether a digest reference points at source content rather than a run output.
fn is_source_blob(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("git-blob:") || value.starts_with("blob:") || value.starts_with("source:")
}

/// Every review that reviews another node must be complete, and independent
/// where the policy requires it. A pending, incomplete, or non-independent
/// review is surfaced so a graph whose review is unfinished can never read as
/// VALID — the honest state is "structurally traced but review pending".
pub(super) fn reviews_complete(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    for id in graph.ids_of_kind(NodeKind::Review) {
        if !reviews_something(graph, id) {
            continue;
        }
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let status = node.attr("status").unwrap_or("<missing>");
        if !policy.review_complete_statuses.iter().any(|s| s == status) {
            findings.push(Finding::new(
                FindingCode::ReviewIncomplete,
                Some(id.clone()),
                format!(
                    "review {id} has status '{status}', not a completed review status (the trace is complete but the review is not)"
                ),
            ));
            // An openly pending review needs no further record findings piled on.
            continue;
        }
        for problem in completed_review_problems(node, policy) {
            findings.push(Finding::new(
                FindingCode::ReviewIncomplete,
                Some(id.clone()),
                format!("review {id}: {problem}"),
            ));
        }
    }
}

/// The ways a review that *claims* completion is not backed by a substantive,
/// closed record: missing reviewer/date/disposition, a disposition that is not a
/// closed outcome, or missing independence. A `status complete` alone is never
/// enough.
fn completed_review_problems(node: &Node, policy: &Policy) -> Vec<String> {
    let mut problems = Vec::new();
    let missing: Vec<&str> = policy
        .review_record_attrs
        .iter()
        .filter(|attr| node.attr(attr).is_none())
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        problems.push(format!(
            "status is complete but the substantive record is missing {}",
            missing.join(", ")
        ));
    }
    if let Some(disposition) = node.attr("disposition") {
        let normalized = disposition.trim().to_ascii_lowercase();
        if !policy.review_closed_dispositions.contains(&normalized) {
            problems.push(format!(
                "disposition '{disposition}' is not a closed review outcome"
            ));
        }
    }
    if policy.review_requires_independence && !is_independent(node) {
        problems.push("review is not marked independent".to_string());
    }
    problems
}

/// Whether `id` has any outgoing `reviews` edge.
fn reviews_something(graph: &Graph, id: &NodeId) -> bool {
    graph
        .edges()
        .any(|e| e.from == *id && e.relation == RelationKind::Reviews)
}

/// Whether a review node records its independence.
fn is_independent(node: &Node) -> bool {
    matches!(
        node.attr("independent"),
        Some("yes" | "true" | "independent")
    )
}

/// Every derived requirement must carry its safety record and a review.
pub(super) fn derived_requirements(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    for id in graph.scope.requirements.iter() {
        if graph.kind_of(id) != Some(NodeKind::DerivedRequirement) {
            continue;
        }
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let mut missing: Vec<String> = policy
            .derived_required_attrs
            .iter()
            .filter(|attr| node.attr(attr).is_none())
            .map(|attr| format!("attribute {attr}"))
            .collect();
        if policy.derived_requires_review && !has_reviewer(graph, id) {
            missing.push("independent review".to_string());
        }
        if !missing.is_empty() {
            findings.push(Finding::new(
                FindingCode::DerivedRequirementIncomplete,
                Some(id.clone()),
                format!("derived requirement {id} is missing {}", missing.join(", ")),
            ));
        }
    }
}

/// Every verification case must carry a resolvable-looking selector.
pub(super) fn selectors_present(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    for id in graph.ids_of_kind(NodeKind::VerificationCase) {
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let has_path = node.locator.as_deref().is_some_and(|p| !p.is_empty());
        let has_test = node.attr(&policy.selector_attr).is_some();
        if !has_path || !has_test {
            findings.push(Finding::new(
                FindingCode::UnresolvedSelector,
                Some(id.clone()),
                format!(
                    "verification case {id} has no test selector (needs a locator path and a {} attribute)",
                    policy.selector_attr
                ),
            ));
        }
    }
}

fn kinds_present(graph: &Graph, ids: &BTreeSet<NodeId>) -> BTreeSet<NodeKind> {
    ids.iter().filter_map(|id| graph.kind_of(id)).collect()
}

/// The `to` of the first outgoing edge of `relation` from `id` whose target has
/// `kind`.
fn target_of_kind(
    graph: &Graph,
    id: &NodeId,
    relation: RelationKind,
    kind: NodeKind,
) -> Option<NodeId> {
    outgoing(graph, id)
        .find(|e| e.relation == relation && graph.kind_of(&e.to) == Some(kind))
        .map(|e| e.to.clone())
}

/// Whether `id` has any outgoing edge to a node of `kind`.
fn reaches_kind(graph: &Graph, id: &NodeId, kind: NodeKind) -> bool {
    outgoing(graph, id).any(|e| graph.kind_of(&e.to) == Some(kind))
}

/// Whether the result, or its case, covers an in-scope requirement.
fn covers_scope_requirement(graph: &Graph, result: &NodeId, case: Option<&NodeId>) -> bool {
    let covers_from = |node: &NodeId| {
        outgoing(graph, node)
            .any(|e| e.relation == RelationKind::Covers && graph.scope.requirements.contains(&e.to))
    };
    covers_from(result) || case.is_some_and(covers_from)
}

/// Whether some Review node reviews `id`.
fn has_reviewer(graph: &Graph, id: &NodeId) -> bool {
    graph.edges().any(|e| {
        e.to == *id
            && e.relation == RelationKind::Reviews
            && graph.kind_of(&e.from) == Some(NodeKind::Review)
    })
}

fn outgoing<'a>(graph: &'a Graph, id: &'a NodeId) -> impl Iterator<Item = &'a Edge> {
    graph.edges().filter(move |e| e.from == *id)
}
